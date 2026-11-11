//! A pannable, zoomable drawing surface for diagram models.

use super::draw::{self, set};
use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub enum Reaction {
    None,
    Redraw,
    /// Open a file (path, line) or URL.
    Open(String, Option<u32>),
}

pub trait Model: 'static {
    /// Content size in diagram units.
    fn size(&self) -> (f64, f64);
    fn draw(&self, cr: &cairo::Context, w: &gtk::DrawingArea);
    fn hover(&mut self, _p: Option<(f64, f64)>) -> bool {
        false
    }
    fn tooltip(&self, _p: (f64, f64)) -> Option<String> {
        None
    }
    fn click(&mut self, _p: (f64, f64), _n: i32, _mods: gdk::ModifierType) -> Reaction {
        Reaction::None
    }
    /// Start dragging an item under p (true) instead of panning.
    fn grab(&mut self, _p: (f64, f64)) -> bool {
        false
    }
    fn drag(&mut self, _dx: f64, _dy: f64) {}
    fn release(&mut self) {}
}

struct St {
    scale: Cell<f64>,
    ox: Cell<f64>,
    oy: Cell<f64>,
    fitted: Cell<bool>,
    dragging_item: Cell<bool>,
    start: Cell<(f64, f64)>,
    last: Cell<(f64, f64)>,
    moved: Cell<bool>,
    pointer: Cell<(f64, f64)>,
    pinch_scale: Cell<f64>,
    /// Smallest zoom a fit may choose (readability over seeing everything).
    min_fit: Cell<f64>,
    /// Where to centre after a fit when the whole content doesn't fit.
    anchor: Cell<Option<(f64, f64)>>,
}

pub struct Diagram<M: Model> {
    pub root: gtk::Overlay,
    pub area: gtk::DrawingArea,
    pub model: Rc<RefCell<M>>,
    pub tools: gtk::Box,
    st: Rc<St>,
}

impl<M: Model> Diagram<M> {
    pub fn new(model: M, on_open: Rc<dyn Fn(String, Option<u32>)>) -> Rc<Diagram<M>> {
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_has_tooltip(true);
        let root = gtk::Overlay::new();
        root.set_child(Some(&area));

        let zoom = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        zoom.add_css_class("linked");
        zoom.add_css_class("osd-tools");
        zoom.set_halign(gtk::Align::End);
        zoom.set_valign(gtk::Align::End);
        zoom.set_margin_end(14);
        zoom.set_margin_bottom(14);
        let b_out = gtk::Button::from_icon_name("zoom-out-symbolic");
        let b_fit = gtk::Button::from_icon_name("zoom-fit-best-symbolic");
        let b_in = gtk::Button::from_icon_name("zoom-in-symbolic");
        b_out.set_tooltip_text(Some("Zoom out (Ctrl+scroll)"));
        b_fit.set_tooltip_text(Some("Fit to view"));
        b_in.set_tooltip_text(Some("Zoom in"));
        for b in [&b_out, &b_fit, &b_in] {
            zoom.append(b);
        }
        root.add_overlay(&zoom);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tools.set_halign(gtk::Align::Start);
        tools.set_valign(gtk::Align::Start);
        tools.set_margin_start(12);
        tools.set_margin_top(10);
        root.add_overlay(&tools);

        let d = Rc::new(Diagram {
            root,
            area,
            model: Rc::new(RefCell::new(model)),
            tools,
            st: Rc::new(St {
                scale: Cell::new(1.0),
                ox: Cell::new(0.0),
                oy: Cell::new(0.0),
                fitted: Cell::new(false),
                dragging_item: Cell::new(false),
                start: Cell::new((0.0, 0.0)),
                last: Cell::new((0.0, 0.0)),
                moved: Cell::new(false),
                pointer: Cell::new((0.0, 0.0)),
                pinch_scale: Cell::new(1.0),
                min_fit: Cell::new(0.15),
                anchor: Cell::new(None),
            }),
        });

        let (m, st) = (d.model.clone(), d.st.clone());
        let me = Rc::downgrade(&d);
        d.area.set_draw_func(move |a, cr, w, h| {
            let t = draw::theme();
            set(cr, t.bg, 1.0);
            let _ = cr.paint();
            if !st.fitted.get() && w > 0 && h > 0 {
                if let Some(me) = me.upgrade() {
                    me.fit_to(w as f64, h as f64);
                }
            }
            let s = st.scale.get();
            // dot grid
            let step = 22.0 * s;
            if step > 6.0 {
                set(cr, t.fg, if t.dark { 0.07 } else { 0.09 });
                let (ox, oy) = (st.ox.get().rem_euclid(step), st.oy.get().rem_euclid(step));
                let mut y = oy;
                while y < h as f64 {
                    let mut x = ox;
                    while x < w as f64 {
                        cr.rectangle(x, y, 1.2, 1.2);
                        x += step;
                    }
                    y += step;
                }
                let _ = cr.fill();
            }
            cr.save().ok();
            cr.translate(st.ox.get(), st.oy.get());
            cr.scale(s, s);
            m.borrow().draw(cr, a);
            cr.restore().ok();
        });

        // pan / item drag
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let (m, st, a) = (d.model.clone(), d.st.clone(), d.area.clone());
        let to_c = Self::to_content_fn(&d.st);
        drag.connect_drag_begin(move |_, x, y| {
            st.moved.set(false);
            st.start.set((st.ox.get(), st.oy.get()));
            st.last.set((0.0, 0.0));
            st.dragging_item.set(m.borrow_mut().grab(to_c(x, y)));
            a.grab_focus();
        });
        let (m, st, a) = (d.model.clone(), d.st.clone(), d.area.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            if dx.abs() + dy.abs() > 3.0 {
                st.moved.set(true);
            }
            if st.dragging_item.get() {
                let (lx, ly) = st.last.get();
                let s = st.scale.get();
                m.borrow_mut().drag((dx - lx) / s, (dy - ly) / s);
                st.last.set((dx, dy));
            } else {
                let (sx, sy) = st.start.get();
                st.ox.set(sx + dx);
                st.oy.set(sy + dy);
            }
            a.queue_draw();
        });
        let (m, st, a) = (d.model.clone(), d.st.clone(), d.area.clone());
        drag.connect_drag_end(move |_, _, _| {
            if st.dragging_item.get() {
                st.dragging_item.set(false);
                m.borrow_mut().release();
                a.queue_draw();
            }
        });
        d.area.add_controller(drag);

        let click = gtk::GestureClick::new();
        click.set_button(1);
        let (m, st, a) = (d.model.clone(), d.st.clone(), d.area.clone());
        let to_c = Self::to_content_fn(&d.st);
        click.connect_released(move |g, n, x, y| {
            if st.moved.get() {
                return;
            }
            let mods = g.current_event_state();
            let r = m.borrow_mut().click(to_c(x, y), n, mods);
            match r {
                Reaction::None => {}
                Reaction::Redraw => a.queue_draw(),
                Reaction::Open(p, l) => on_open(p, l),
            }
        });
        d.area.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        let (m, st, a) = (d.model.clone(), d.st.clone(), d.area.clone());
        let to_c = Self::to_content_fn(&d.st);
        motion.connect_motion(move |_, x, y| {
            st.pointer.set((x, y));
            if !st.dragging_item.get() && m.borrow_mut().hover(Some(to_c(x, y))) {
                a.queue_draw();
            }
        });
        let (m, a) = (d.model.clone(), d.area.clone());
        motion.connect_leave(move |_| {
            if m.borrow_mut().hover(None) {
                a.queue_draw();
            }
        });
        d.area.add_controller(motion);

        let (m, _st) = (d.model.clone(), d.st.clone());
        let to_c = Self::to_content_fn(&d.st);
        d.area.connect_query_tooltip(move |_, x, y, _, tip| match m.borrow().tooltip(to_c(x as f64, y as f64)) {
            Some(t) => {
                tip.set_text(Some(&t));
                true
            }
            None => false,
        });

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let me = Rc::downgrade(&d);
        scroll.connect_scroll(move |c, dx, dy| {
            let Some(me) = me.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK);
            let px = c.unit() == gdk::ScrollUnit::Surface;
            if ctrl {
                let f = if px { (1.0 - dy * 0.01).clamp(0.8, 1.25) } else { 1.15f64.powf(-dy) };
                let (x, y) = me.st.pointer.get();
                me.zoom_at(f, x, y);
            } else {
                let k = if px { 1.0 } else { 40.0 };
                me.st.ox.set(me.st.ox.get() - dx * k);
                me.st.oy.set(me.st.oy.get() - dy * k);
                me.area.queue_draw();
            }
            glib::Propagation::Stop
        });
        d.area.add_controller(scroll);

        let pinch = gtk::GestureZoom::new();
        let me = Rc::downgrade(&d);
        pinch.connect_begin(move |_, _| {
            if let Some(me) = me.upgrade() {
                me.st.pinch_scale.set(1.0);
            }
        });
        let me = Rc::downgrade(&d);
        pinch.connect_scale_changed(move |g, s| {
            let Some(me) = me.upgrade() else { return };
            let f = s / me.st.pinch_scale.get();
            me.st.pinch_scale.set(s);
            let (x, y) = g.bounding_box_center().unwrap_or(me.st.pointer.get());
            me.zoom_at(f, x, y);
        });
        d.area.add_controller(pinch);

        let me = Rc::downgrade(&d);
        b_in.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.zoom_center(1.25);
            }
        });
        let me = Rc::downgrade(&d);
        b_out.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.zoom_center(0.8);
            }
        });
        let me = Rc::downgrade(&d);
        b_fit.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.fit();
            }
        });
        d
    }

    fn to_content_fn(st: &Rc<St>) -> impl Fn(f64, f64) -> (f64, f64) + 'static {
        let st = st.clone();
        move |x, y| {
            let s = st.scale.get();
            ((x - st.ox.get()) / s, (y - st.oy.get()) / s)
        }
    }

    fn fit_to(&self, w: f64, h: f64) {
        let (cw, ch) = self.model.borrow().size();
        let (cw, ch) = (cw.max(1.0), ch.max(1.0));
        let fit = ((w - 60.0) / cw).min((h - 80.0) / ch);
        let s = fit.clamp(self.st.min_fit.get().min(1.0), 1.4);
        self.st.scale.set(s);
        if s > fit + 1e-6 {
            // doesn't fit at a readable size: start at the anchor (or the left middle)
            let (ax, ay) = self.st.anchor.get().unwrap_or(((w / 2.0 - 30.0) / s, ch / 2.0));
            self.st.ox.set(w / 2.0 - ax * s);
            self.st.oy.set(h / 2.0 - ay * s);
        } else {
            self.st.ox.set((w - cw * s) / 2.0);
            self.st.oy.set(((h - ch * s) / 2.0).max(16.0));
        }
        self.st.fitted.set(true);
    }

    pub fn fit(&self) {
        let (w, h) = (self.area.width() as f64, self.area.height() as f64);
        if w > 0.0 && h > 0.0 {
            self.fit_to(w, h);
        } else {
            self.st.fitted.set(false);
        }
        self.area.queue_draw();
    }

    /// Fit again on the next draw (after the content changed a lot).
    pub fn refit(&self) {
        self.st.fitted.set(false);
        self.area.queue_draw();
    }

    fn zoom_at(&self, f: f64, x: f64, y: f64) {
        let s0 = self.st.scale.get();
        let s1 = (s0 * f).clamp(0.08, 4.0);
        let k = s1 / s0;
        self.st.ox.set(x - (x - self.st.ox.get()) * k);
        self.st.oy.set(y - (y - self.st.oy.get()) * k);
        self.st.scale.set(s1);
        self.area.queue_draw();
    }

    fn zoom_center(&self, f: f64) {
        self.zoom_at(f, self.area.width() as f64 / 2.0, self.area.height() as f64 / 2.0);
    }

    /// Bring a content point to the middle of the view.
    pub fn center_on(&self, x: f64, y: f64) {
        let s = self.st.scale.get();
        self.st.ox.set(self.area.width() as f64 / 2.0 - x * s);
        self.st.oy.set(self.area.height() as f64 / 2.0 - y * s);
        self.area.queue_draw();
    }

    /// Fits never zoom out below `min`; the view then starts centred on `anchor`.
    pub fn set_fit(&self, min: f64, anchor: Option<(f64, f64)>) {
        self.st.min_fit.set(min);
        self.st.anchor.set(anchor);
    }

    pub fn redraw(&self) {
        self.area.queue_draw();
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
}
