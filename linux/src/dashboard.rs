//! Dashboard: KPIs, usage-limit history, spend per day, models, projects, accounts, activity.
//! Charts are drawn with cairo.

use crate::deck::Deck;
use crate::model::{self, account_label, money, parse_ts, tokens};
use crate::widgets::{self, fg, label, set, Rgb, ACCENT, RED};
use adw::prelude::*;
use chrono::{DateTime, Duration as CDuration, Local, NaiveDate, Utc};
use claude_deck::stats::Stats;
use gtk::{cairo, gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

pub struct Dashboard {
    pub root: gtk::Box,
    scroll: gtk::ScrolledWindow,
    content: gtk::Box,
    subtitle: gtk::Label,
    reload: gtk::Button,
    deck: RefCell<Weak<Deck>>,
    days: Cell<u32>,
    window: Cell<u8>,
    loading: Cell<bool>,
    loaded_at: Cell<Option<Instant>>,
    stats: RefCell<Option<Rc<Stats>>>,
    error: RefCell<Option<String>>,
}

struct Ctx {
    names: Vec<String>,
    accounts: Vec<model::AccountInfo>,
    threshold: f64,
    weekly: f64,
}

impl Dashboard {
    pub fn new() -> Rc<Dashboard> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.set_margin_top(12);
        bar.set_margin_bottom(4);
        bar.set_margin_start(28);
        bar.set_margin_end(28);
        let subtitle = label("", &["dim-label"]);
        subtitle.set_hexpand(true);
        bar.append(&subtitle);
        let period = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        period.add_css_class("linked");
        let mut first: Option<gtk::ToggleButton> = None;
        let mut toggles = vec![];
        for (d, l) in [(7u32, "7D"), (14, "14D"), (30, "30D"), (90, "90D")] {
            let t = gtk::ToggleButton::with_label(l);
            if let Some(f) = &first {
                t.set_group(Some(f));
            } else {
                first = Some(t.clone());
            }
            t.set_active(d == 14);
            period.append(&t);
            toggles.push((d, t));
        }
        bar.append(&period);
        let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
        reload.set_tooltip_text(Some("Reload"));
        bar.append(&reload);
        root.append(&bar);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 28);
        content.set_margin_top(12);
        content.set_margin_bottom(28);
        content.set_margin_start(28);
        content.set_margin_end(28);
        let clamp = adw::Clamp::builder().maximum_size(1180).tightening_threshold(900).child(&content).build();
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&clamp));
        root.append(&scroll);

        let d = Rc::new(Dashboard {
            root,
            scroll: scroll.clone(),
            content,
            subtitle,
            reload: reload.clone(),
            deck: RefCell::new(Weak::new()),
            days: Cell::new(14),
            window: Cell::new(0),
            loading: Cell::new(false),
            loaded_at: Cell::new(None),
            stats: RefCell::new(None),
            error: RefCell::new(None),
        });
        for (days, t) in toggles {
            let me = Rc::downgrade(&d);
            t.connect_toggled(move |t| {
                if let (true, Some(me)) = (t.is_active(), me.upgrade()) {
                    me.days.set(days);
                    me.load();
                }
            });
        }
        let me = Rc::downgrade(&d);
        reload.connect_clicked(move |_| {
            if let Some(me) = me.upgrade() {
                me.load();
            }
        });
        d.render();
        d
    }

    /// Development aid for snapshots.
    pub fn scroll_to(&self, frac: f64) {
        let a = self.scroll.vadjustment();
        a.set_value((a.upper() - a.page_size()) * frac);
    }

    pub fn set_deck(&self, deck: Weak<Deck>) {
        *self.deck.borrow_mut() = deck;
    }

    pub fn load_if_stale(self: &Rc<Self>) {
        if self.loaded_at.get().is_none_or(|t| t.elapsed().as_secs() > 60) {
            self.load();
        }
    }

    pub fn load(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        self.loading.set(true);
        self.reload.set_sensitive(false);
        if self.stats.borrow().is_none() {
            self.render();
        }
        let days = self.days.get();
        let me = self.clone();
        glib::spawn_future_local(async move {
            let res = gio::spawn_blocking(move || claude_deck::stats::compute(days).map_err(|e| format!("{e:#}"))).await;
            me.loading.set(false);
            me.reload.set_sensitive(true);
            match res {
                Ok(Ok(s)) => {
                    *me.stats.borrow_mut() = Some(Rc::new(s));
                    *me.error.borrow_mut() = None;
                    me.loaded_at.set(Some(Instant::now()));
                }
                Ok(Err(e)) => *me.error.borrow_mut() = Some(e),
                Err(_) => *me.error.borrow_mut() = Some("statistics failed".into()),
            }
            me.render();
        });
    }

    /// Re-render with the latest account list / limits (e.g. after a usage refresh or theme change).
    pub fn redraw(self: &Rc<Self>) {
        if self.root.is_mapped() {
            self.render();
        }
    }

    fn ctx(&self) -> Ctx {
        match self.deck.borrow().upgrade() {
            Some(d) => {
                let cfg = d.config.borrow();
                Ctx {
                    names: d.account_names(),
                    accounts: d.accounts.borrow().clone(),
                    threshold: cfg.threshold,
                    weekly: cfg.weekly_threshold,
                }
            }
            None => Ctx { names: vec![], accounts: vec![], threshold: 90.0, weekly: 95.0 },
        }
    }

    fn render(self: &Rc<Self>) {
        while let Some(c) = self.content.first_child() {
            self.content.remove(&c);
        }
        let stats = self.stats.borrow().clone();
        let Some(stats) = stats else {
            if let Some(e) = self.error.borrow().as_ref() {
                let p = adw::StatusPage::builder()
                    .icon_name("dialog-warning-symbolic")
                    .title("Couldn't load statistics")
                    .description(glib::markup_escape_text(e).as_str())
                    .build();
                self.content.append(&p);
            } else {
                let sp = gtk::Spinner::new();
                sp.set_size_request(32, 32);
                sp.set_margin_top(160);
                sp.start();
                self.content.append(&sp);
            }
            self.subtitle.set_text("");
            return;
        };
        let ctx = self.ctx();
        self.subtitle.set_text(&format!(
            "Last {} days · updated {}",
            stats.days,
            self.loaded_at
                .get()
                .map(|t| (Local::now() - CDuration::from_std(t.elapsed()).unwrap_or_default()).format("%H:%M").to_string())
                .unwrap_or_default()
        ));

        self.content.append(&kpi_row(&stats));

        let s = section("Usage limits", Some("How close each account is to its caps. CC Deck switches at the dashed line."));
        s.append(&self.limits_card(&stats, &ctx));
        self.content.append(&s);

        let s = section("Spend", Some("Estimated at API list prices from local Claude Code transcripts."));
        s.append(&cost_card(&stats, &ctx));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        row.set_homogeneous(true);
        row.append(&models_card(&stats));
        row.append(&projects_card(&stats));
        s.append(&row);
        self.content.append(&s);

        let s = section("Accounts", None);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        row.set_homogeneous(true);
        row.append(&accounts_card(&stats, &ctx));
        row.append(&activity_card(&stats));
        s.append(&row);
        self.content.append(&s);

        let foot = label(
            "Costs are estimates; subscription plans are not billed per token. Sessions from before account tracking are shown as “Before tracking”.",
            &["caption", "dim-label"],
        );
        foot.set_wrap(true);
        self.content.append(&foot);
    }

    fn limits_card(self: &Rc<Self>, stats: &Stats, ctx: &Ctx) -> gtk::Box {
        let c = card(None, None);
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 24);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 14);
        left.set_size_request(270, -1);
        left.set_valign(gtk::Align::Center);
        left.append(&label("Right now", &["heading"]));
        for acc in &ctx.accounts {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
            row.append(&widgets::rings(acc.five_pct(), acc.week_pct(), ctx.threshold, ctx.weekly, 56, 6.0));
            let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
            v.set_valign(gtk::Align::Center);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            top.append(&widgets::dot(widgets::account_color(&acc.name, &ctx.names), 8));
            top.append(&label(&acc.name, &["heading"]));
            if let Some(p) = acc.plan_label() {
                top.append(&widgets::badge(&p));
            }
            v.append(&top);
            if acc.is_ok() {
                for (l, w, th) in [("5h", acc.five(), ctx.threshold), ("7d", acc.week(), ctx.weekly)] {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    line.append(&label(l, &["dim-label", "numeric"]));
                    let pct = w.map(|w| w.current_pct());
                    let val = label(&pct.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "–".into()), &["heading", "numeric"]);
                    if let Some(p) = pct {
                        if p >= th {
                            val.add_css_class("error");
                        } else if p >= th * 0.75 {
                            val.add_css_class("warning");
                        }
                    }
                    line.append(&val);
                    if let Some(t) = w.and_then(|w| w.resets_at) {
                        line.append(&label(&format!("· resets in {}", claude_deck::usage::until(t)), &["dim-label", "numeric"]));
                    }
                    v.append(&line);
                }
            } else {
                v.append(&label(&acc.state_label(), &["dim-label"]));
            }
            row.append(&v);
            left.append(&row);
        }
        h.append(&left);
        h.append(&gtk::Separator::new(gtk::Orientation::Vertical));

        let right = gtk::Box::new(gtk::Orientation::Vertical, 14);
        right.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let tt = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tt.set_hexpand(true);
        tt.append(&label("History", &["heading"]));
        let win = self.window.get();
        tt.append(&label(if win == 0 { "5-hour window, % used" } else { "7-day window, % used" }, &["dim-label"]));
        top.append(&tt);
        let seg = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        seg.add_css_class("linked");
        seg.set_valign(gtk::Align::Center);
        let b5 = gtk::ToggleButton::with_label("5 hours");
        let b7 = gtk::ToggleButton::with_label("7 days");
        b7.set_group(Some(&b5));
        b5.set_active(win == 0);
        b7.set_active(win == 1);
        seg.append(&b5);
        seg.append(&b7);
        top.append(&seg);
        right.append(&top);

        let mut series: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        for s in &stats.usage_history {
            let key = if win == 0 { "five" } else { "week" };
            let (Some(ts), Some(acc), Some(v)) = (
                s.get("ts").and_then(|t| t.as_str()).and_then(parse_ts),
                s.get("account").and_then(|a| a.as_str()),
                s.get(key).and_then(|v| v.as_f64()),
            ) else {
                continue;
            };
            series.entry(acc.to_string()).or_default().push((ts.timestamp() as f64, v));
        }
        if series.is_empty() {
            right.append(&empty_chart("Samples are recorded every few minutes while CC Deck is open."));
        } else {
            let line = if win == 0 { ctx.threshold } else { ctx.weekly };
            let names = ctx.names.clone();
            let mut lines: Vec<(String, Rgb, Vec<(f64, f64)>)> = series
                .into_iter()
                .map(|(n, mut pts)| {
                    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let c = widgets::account_color(&n, &names);
                    (n, c, pts)
                })
                .collect();
            lines.sort_by_key(|l| names.iter().position(|n| *n == l.0).unwrap_or(99));
            right.append(&history_chart(lines.clone(), line));
            right.append(&legend(lines.iter().map(|l| (l.0.clone(), l.1)).collect()));
        }
        h.append(&right);
        c.append(&h);

        for (b, w) in [(b5, 0u8), (b7, 1u8)] {
            let me = Rc::downgrade(self);
            b.connect_toggled(move |b| {
                if let (true, Some(me)) = (b.is_active(), me.upgrade()) {
                    if me.window.get() != w {
                        me.window.set(w);
                        let me2 = me.clone();
                        glib::idle_add_local_once(move || me2.render());
                    }
                }
            });
        }
        c
    }
}

// ---------- layout helpers ----------

fn section(title: &str, subtitle: Option<&str>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let h = gtk::Box::new(gtk::Orientation::Vertical, 2);
    h.append(&label(title, &["dash-title"]));
    if let Some(s) = subtitle {
        h.append(&label(s, &["dim-label"]));
    }
    b.append(&h);
    b
}

fn card(title: Option<&str>, subtitle: Option<&str>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 14);
    b.add_css_class("card");
    b.add_css_class("deck-card");
    if let Some(t) = title {
        let h = gtk::Box::new(gtk::Orientation::Vertical, 2);
        h.append(&label(t, &["heading"]));
        if let Some(s) = subtitle {
            h.append(&label(s, &["dim-label"]));
        }
        b.append(&h);
    }
    b
}

fn empty_chart(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("dim-label");
    l.set_size_request(-1, 140);
    l.set_wrap(true);
    l
}

fn legend(items: Vec<(String, Rgb)>) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_max_children_per_line(8);
    f.set_column_spacing(14);
    f.set_halign(gtk::Align::Start);
    for (n, c) in items {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        b.append(&widgets::dot(c, 8));
        b.append(&label(&account_label(&n), &["caption"]));
        f.insert(&b, -1);
    }
    f
}

fn text(area: &gtk::DrawingArea, cr: &cairo::Context, s: &str, x: f64, y: f64, align: f64, scale: f64, bold: bool, c: Rgb, a: f64) -> (f64, f64) {
    let layout = area.create_pango_layout(Some(s));
    let mut fd = area.pango_context().font_description().unwrap_or_default();
    fd.set_size((fd.size() as f64 * scale) as i32);
    if bold {
        fd.set_weight(gtk::pango::Weight::Semibold);
    }
    layout.set_font_description(Some(&fd));
    let (w, h) = layout.pixel_size();
    cr.move_to(x - w as f64 * align, y);
    set(cr, c, a);
    pangocairo::functions::show_layout(cr, &layout);
    (w as f64, h as f64)
}

fn measure(area: &gtk::DrawingArea, s: &str, scale: f64, bold: bool) -> (f64, f64) {
    let layout = area.create_pango_layout(Some(s));
    let mut fd = area.pango_context().font_description().unwrap_or_default();
    fd.set_size((fd.size() as f64 * scale) as i32);
    if bold {
        fd.set_weight(gtk::pango::Weight::Semibold);
    }
    layout.set_font_description(Some(&fd));
    let (w, h) = layout.pixel_size();
    (w as f64, h as f64)
}

/// A floating box with a title and (colour, label, value) rows, kept inside the area.
fn hover_card(area: &gtk::DrawingArea, cr: &cairo::Context, x: f64, y: f64, w_area: f64, title: &str, rows: &[(Rgb, String, String)]) {
    let pad = 10.0;
    let (tw, th) = measure(area, title, 0.85, true);
    let mut width = tw;
    let mut lh = 0.0f64;
    for (_, l, v) in rows {
        let (lw, h1) = measure(area, l, 0.85, false);
        let (vw, _) = measure(area, v, 0.85, true);
        width = width.max(14.0 + lw + 16.0 + vw);
        lh = lh.max(h1);
    }
    let width = width.max(140.0) + pad * 2.0;
    let height = pad * 2.0 + th + rows.len() as f64 * (lh + 4.0) + 2.0;
    let mut bx = x + 12.0;
    if bx + width > w_area - 4.0 {
        bx = x - 12.0 - width;
    }
    let bx = bx.max(4.0);
    let dark = widgets::is_dark();
    set(cr, if dark { widgets::hex(0x303030) } else { widgets::hex(0xFFFFFF) }, 0.97);
    widgets::rounded_rect(cr, bx, y, width, height, 10.0);
    let _ = cr.fill_preserve();
    set(cr, fg(area), 0.15);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let ink = fg(area);
    text(area, cr, title, bx + pad, y + pad, 0.0, 0.85, true, ink, 1.0);
    let mut yy = y + pad + th + 4.0;
    for (c, l, v) in rows {
        set(cr, *c, 1.0);
        cr.arc(bx + pad + 4.0, yy + lh / 2.0, 3.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        text(area, cr, l, bx + pad + 14.0, yy, 0.0, 0.85, false, ink, 0.85);
        text(area, cr, v, bx + width - pad, yy, 1.0, 0.85, true, ink, 1.0);
        yy += lh + 4.0;
    }
}

fn hoverable(area: &gtk::DrawingArea) -> Rc<Cell<Option<f64>>> {
    let hover = Rc::new(Cell::new(None));
    let m = gtk::EventControllerMotion::new();
    let (h1, a1) = (hover.clone(), area.clone());
    m.connect_motion(move |_, x, _| {
        h1.set(Some(x));
        a1.queue_draw();
    });
    let (h2, a2) = (hover.clone(), area.clone());
    m.connect_leave(move |_| {
        h2.set(None);
        a2.queue_draw();
    });
    area.add_controller(m);
    hover
}

fn dashed(cr: &cairo::Context, x0: f64, y0: f64, x1: f64, y1: f64, c: Rgb, a: f64, dash: f64) {
    set(cr, c, a);
    cr.set_line_width(1.0);
    cr.set_dash(&[dash, dash], 0.0);
    cr.move_to(x0, y0);
    cr.line_to(x1, y1);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

/// 1, 2, 2.5, 5 × 10^k at or above v.
fn nice(v: f64) -> f64 {
    if v <= 0.0 {
        return 1.0;
    }
    let p = 10f64.powf(v.log10().floor());
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if m * p >= v {
            return m * p;
        }
    }
    10.0 * p
}

// ---------- KPIs ----------

fn day_totals(stats: &Stats) -> BTreeMap<String, (f64, u64)> {
    let mut m: BTreeMap<String, (f64, u64)> = BTreeMap::new();
    for r in &stats.daily {
        let e = m.entry(r.date.clone()).or_default();
        e.0 += r.cost;
        e.1 += r.tokens;
    }
    m
}

fn kpi_row(stats: &Stats) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.set_column_spacing(16);
    g.set_row_spacing(16);
    g.set_column_homogeneous(true);
    let totals = day_totals(stats);
    let t = &stats.totals;
    let tiles = [
        kpi("Estimated cost", "emblem-documents-symbolic", &money(t.cost), &format!("{} today", money(t.today_cost)), totals.values().map(|v| v.0).collect()),
        kpi("Tokens", "view-list-ordered-symbolic", &tokens(t.tokens), &format!("{} today", tokens(t.today_tokens)), totals.values().map(|v| v.1 as f64).collect()),
        kpi("Sessions", "chat-message-new-symbolic", &t.sessions.to_string(), &format!("{} responses", t.messages), vec![]),
        kpi("Active time", "preferences-system-time-symbolic", &model::minutes(t.active_minutes), "across all accounts", vec![]),
    ];
    for (i, tile) in tiles.iter().enumerate() {
        g.attach(tile, i as i32, 0, 1, 1);
    }
    g
}

fn kpi(title: &str, icon: &str, value: &str, detail: &str, spark: Vec<f64>) -> gtk::Box {
    let c = card(None, None);
    c.set_spacing(6);
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("dim-label");
    h.append(&i);
    let t = label(title, &["dim-label"]);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    h.append(&t);
    c.append(&h);
    let v = label(value, &["kpi-value"]);
    v.set_ellipsize(gtk::pango::EllipsizeMode::End);
    c.append(&v);
    c.append(&label(detail, &["caption", "dim-label"]));
    let area = gtk::DrawingArea::new();
    area.set_content_height(30);
    area.set_margin_top(4);
    if spark.len() > 1 {
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let max = spark.iter().cloned().fold(0.0, f64::max).max(1e-9);
            let n = spark.len() as f64 - 1.0;
            let pt = |i: usize, v: f64| (i as f64 / n * w, h - 2.0 - (v / max) * (h - 4.0));
            cr.move_to(0.0, h);
            for (i, v) in spark.iter().enumerate() {
                let (x, y) = pt(i, *v);
                cr.line_to(x, y);
            }
            cr.line_to(w, h);
            cr.close_path();
            set(cr, ACCENT, 0.16);
            let _ = cr.fill();
            for (i, v) in spark.iter().enumerate() {
                let (x, y) = pt(i, *v);
                if i == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            set(cr, ACCENT, 1.0);
            cr.set_line_width(1.8);
            cr.set_line_join(cairo::LineJoin::Round);
            let _ = cr.stroke();
        });
    }
    c.append(&area);
    c
}

// ---------- limit history ----------

fn history_chart(lines: Vec<(String, Rgb, Vec<(f64, f64)>)>, threshold: f64) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(220);
    area.set_hexpand(true);
    let hover = hoverable(&area);
    area.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let (ml, mr, mt, mb) = (6.0, 44.0, 14.0, 24.0);
        let (pw, ph) = (w - ml - mr, h - mt - mb);
        if pw <= 10.0 || ph <= 10.0 {
            return;
        }
        let ink = fg(a);
        let t1 = lines.iter().flat_map(|l| l.2.iter().map(|p| p.0)).fold(f64::MIN, f64::max);
        let mut t0 = lines.iter().flat_map(|l| l.2.iter().map(|p| p.0)).fold(f64::MAX, f64::min);
        if t1 - t0 < 3600.0 {
            t0 = t1 - 3600.0;
        }
        let x = |t: f64| ml + (t - t0) / (t1 - t0) * pw;
        let y = |v: f64| mt + ph * (1.0 - v.clamp(0.0, 100.0) / 100.0);
        cr.set_line_width(1.0);
        for v in [0.0, 25.0, 50.0, 75.0, 100.0] {
            set(cr, ink, 0.10);
            cr.move_to(ml, y(v).round() + 0.5);
            cr.line_to(ml + pw, y(v).round() + 0.5);
            let _ = cr.stroke();
            text(a, cr, &format!("{v:.0}%"), w - 2.0, y(v) - 7.0, 1.0, 0.8, false, ink, 0.55);
        }
        let span = t1 - t0;
        let fmt = if span > 2.0 * 86400.0 { "%b %-d" } else if span > 86400.0 { "%a %H:%M" } else { "%H:%M" };
        for i in 0..5 {
            let t = t0 + span * i as f64 / 4.0;
            let lbl = DateTime::<Utc>::from_timestamp(t as i64, 0)
                .map(|d| d.with_timezone(&Local).format(fmt).to_string())
                .unwrap_or_default();
            let align = if i == 0 { 0.0 } else if i == 4 { 1.0 } else { 0.5 };
            text(a, cr, &lbl, x(t), mt + ph + 6.0, align, 0.8, false, ink, 0.55);
        }
        dashed(cr, ml, y(threshold), ml + pw, y(threshold), RED, 0.7, 4.0);
        text(a, cr, &format!("switch at {threshold:.0}%"), ml + pw, y(threshold) - 16.0, 1.0, 0.75, false, RED, 0.9);

        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        for (_, c, pts) in &lines {
            set(cr, *c, 1.0);
            cr.set_line_width(2.0);
            let mut prev: Option<f64> = None;
            for (t, v) in pts {
                if prev.is_none_or(|p| t - p > 1800.0) {
                    if prev.is_some() {
                        let _ = cr.stroke();
                    }
                    cr.move_to(x(*t), y(*v));
                    if pts.len() == 1 {
                        cr.line_to(x(*t) + 0.1, y(*v));
                    }
                } else {
                    cr.line_to(x(*t), y(*v));
                }
                prev = Some(*t);
            }
            let _ = cr.stroke();
        }

        if let Some(hx) = hover.get().filter(|hx| *hx >= ml && *hx <= ml + pw) {
            let t = t0 + (hx - ml) / pw * span;
            set(cr, ink, 0.3);
            cr.move_to(hx.round() + 0.5, mt);
            cr.line_to(hx.round() + 0.5, mt + ph);
            let _ = cr.stroke();
            let mut rows = vec![];
            for (n, c, pts) in &lines {
                if let Some(p) = pts.iter().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs())) {
                    if (p.0 - t).abs() < 3.0 * 3600.0 {
                        rows.push((*c, account_label(n), format!("{:.0}%", p.1)));
                        set(cr, *c, 1.0);
                        cr.arc(x(p.0), y(p.1), 3.5, 0.0, std::f64::consts::TAU);
                        let _ = cr.fill();
                    }
                }
            }
            let title = DateTime::<Utc>::from_timestamp(t as i64, 0)
                .map(|d| d.with_timezone(&Local).format("%b %-d, %H:%M").to_string())
                .unwrap_or_default();
            hover_card(a, cr, hx, mt, w, &title, &rows);
        }
    });
    area
}

// ---------- spend ----------

fn cost_card(stats: &Stats, ctx: &Ctx) -> gtk::Box {
    let avg = stats.totals.cost / stats.days.max(1) as f64;
    let c = card(
        Some("Estimated cost per day"),
        Some(&format!("{} total · {} per day on average", money(stats.totals.cost), money(avg))),
    );
    if stats.daily.is_empty() {
        c.append(&empty_chart("No Claude Code activity in this period."));
        return c;
    }
    let today = Local::now().date_naive();
    let days: Vec<NaiveDate> = (0..stats.days as i64).rev().map(|i| today - CDuration::days(i)).collect();
    let rank = |n: &str| ctx.names.iter().position(|x| x == n).unwrap_or(99);
    let mut names: Vec<String> = stats.daily.iter().map(|r| r.account.clone()).collect();
    names.sort_by(|a, b| (rank(a), a).cmp(&(rank(b), b)));
    names.dedup();
    let colors: Vec<Rgb> = names.iter().map(|n| widgets::account_color(n, &ctx.names)).collect();
    // cost[day][account]
    let mut cost = vec![vec![0.0f64; names.len()]; days.len()];
    for r in &stats.daily {
        let (Ok(d), Some(ai)) = (NaiveDate::parse_from_str(&r.date, "%Y-%m-%d"), names.iter().position(|n| *n == r.account)) else {
            continue;
        };
        if let Some(di) = days.iter().position(|x| *x == d) {
            cost[di][ai] += r.cost;
        }
    }

    let area = gtk::DrawingArea::new();
    area.set_content_height(230);
    area.set_hexpand(true);
    let hover = hoverable(&area);
    let (names2, colors2) = (names.clone(), colors.clone());
    area.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let (ml, mr, mt, mb) = (6.0, 52.0, 10.0, 24.0);
        let (pw, ph) = (w - ml - mr, h - mt - mb);
        if pw <= 10.0 || ph <= 10.0 {
            return;
        }
        let ink = fg(a);
        let totals: Vec<f64> = cost.iter().map(|d| d.iter().sum()).collect();
        let ymax = nice(totals.iter().cloned().fold(0.0, f64::max).max(avg) * 1.1);
        let y = |v: f64| mt + ph * (1.0 - v / ymax);
        cr.set_line_width(1.0);
        for i in 0..=4 {
            let v = ymax * i as f64 / 4.0;
            set(cr, ink, 0.10);
            cr.move_to(ml, y(v).round() + 0.5);
            cr.line_to(ml + pw, y(v).round() + 0.5);
            let _ = cr.stroke();
            text(a, cr, &money(v), w - 2.0, y(v) - 7.0, 1.0, 0.8, false, ink, 0.55);
        }
        let n = days.len() as f64;
        let slot = pw / n;
        let bw = (slot * 0.62).max(1.0);
        let hi = hover.get().filter(|hx| *hx >= ml && *hx < ml + pw).map(|hx| ((hx - ml) / slot) as usize);
        if let Some(i) = hi {
            set(cr, ink, 0.06);
            cr.rectangle(ml + slot * i as f64, mt, slot, ph);
            let _ = cr.fill();
        }
        for (di, row) in cost.iter().enumerate() {
            let x0 = ml + slot * di as f64 + (slot - bw) / 2.0;
            let mut acc = 0.0;
            for (ai, v) in row.iter().enumerate() {
                if *v <= 0.0 {
                    continue;
                }
                let (ya, yb) = (y(acc + v), y(acc));
                set(cr, colors2[ai], 1.0);
                cr.rectangle(x0, ya, bw, (yb - ya).max(0.5));
                let _ = cr.fill();
                acc += v;
            }
        }
        dashed(cr, ml, y(avg), ml + pw, y(avg), ink, 0.45, 3.0);
        text(a, cr, "avg", ml + 2.0, y(avg) - 15.0, 0.0, 0.75, false, ink, 0.6);
        let stride = if days.len() > 30 { 7 } else if days.len() > 14 { 3 } else { 1 };
        for (di, d) in days.iter().enumerate() {
            if (days.len() - 1 - di) % stride != 0 {
                continue;
            }
            let cx = ml + slot * (di as f64 + 0.5);
            text(a, cr, &d.format("%b %-d").to_string(), cx, mt + ph + 6.0, 0.5, 0.75, false, ink, 0.55);
        }
        if let Some(i) = hi.filter(|i| *i < days.len()) {
            let mut rows: Vec<(Rgb, String, String)> = cost[i]
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0.0)
                .map(|(ai, v)| (colors2[ai], account_label(&names2[ai]), money(*v)))
                .collect();
            rows.sort_by(|a, b| b.2.len().cmp(&a.2.len()).then(b.2.cmp(&a.2)));
            let title = format!("{} · {}", days[i].format("%b %-d"), money(totals[i]));
            hover_card(a, cr, ml + slot * (i as f64 + 0.5), mt, w, &title, &rows);
        }
    });
    c.append(&area);
    c.append(&legend(names.into_iter().zip(colors).collect()));
    c
}

fn models_card(stats: &Stats) -> gtk::Box {
    let c = card(Some("By model"), Some("Share of estimated cost"));
    if stats.models.is_empty() {
        c.append(&empty_chart("No model usage yet."));
        return c;
    }
    let total = stats.models.iter().map(|m| m.cost).sum::<f64>().max(1e-4);
    for (i, m) in stats.models.iter().take(6).enumerate() {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 5);
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let n = label(&model::pretty_model(&m.model), &["heading"]);
        n.set_hexpand(true);
        h.append(&n);
        h.append(&label(&money(m.cost), &["numeric"]));
        let p = label(&format!("{:.0}%", m.cost / total * 100.0), &["dim-label", "numeric"]);
        p.set_width_chars(4);
        p.set_xalign(1.0);
        h.append(&p);
        v.append(&h);
        v.append(&widgets::thin_bar(m.cost / total * 100.0, widgets::series(i), 6));
        v.append(&label(&format!("{} tokens · {} responses", tokens(m.tokens), m.messages), &["caption", "dim-label"]));
        c.append(&v);
    }
    c
}

fn projects_card(stats: &Stats) -> gtk::Box {
    let c = card(Some("Top projects"), Some("By estimated cost"));
    if stats.projects.is_empty() {
        c.append(&empty_chart("No projects yet."));
        return c;
    }
    let max = stats.projects.iter().map(|p| p.cost).fold(0.0, f64::max).max(1e-4);
    for p in stats.projects.iter().take(7) {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let i = gtk::Image::from_icon_name("folder-symbolic");
        i.add_css_class("dim-label");
        h.append(&i);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        v.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = if p.project.is_empty() { "(unknown)".to_string() } else { model::last_component(&p.project) };
        let n = label(&name, &["heading"]);
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&n);
        top.append(&label(&money(p.cost), &["numeric"]));
        v.append(&top);
        v.append(&widgets::thin_bar(p.cost / max * 100.0, ACCENT, 4));
        h.append(&v);
        h.set_tooltip_text(Some(&format!(
            "{} · {} sessions · {} tokens",
            model::short_path(&p.project),
            p.sessions,
            tokens(p.tokens)
        )));
        c.append(&h);
    }
    c
}

fn accounts_card(stats: &Stats, ctx: &Ctx) -> gtk::Box {
    let c = card(Some("Per account"), None);
    let g = gtk::Grid::new();
    g.set_column_spacing(18);
    g.set_row_spacing(10);
    for (i, h) in ["Account", "Sessions", "Tokens", "Cost", "Active"].iter().enumerate() {
        g.attach(&label(h, &["caption-heading", "dim-label"]), i as i32, 0, 1, 1);
    }
    g.attach(&gtk::Separator::new(gtk::Orientation::Horizontal), 0, 1, 5, 1);
    for (r, a) in stats.accounts.iter().enumerate() {
        let row = r as i32 + 2;
        let n = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        n.append(&widgets::dot(widgets::account_color(&a.account, &ctx.names), 8));
        n.append(&label(&account_label(&a.account), &["heading"]));
        n.set_hexpand(true);
        g.attach(&n, 0, row, 1, 1);
        for (i, v) in [
            a.sessions.to_string(),
            tokens(a.tokens),
            money(a.cost),
            model::minutes(a.active_minutes),
        ]
        .iter()
        .enumerate()
        {
            g.attach(&label(v, &["numeric"]), i as i32 + 1, row, 1, 1);
        }
    }
    if stats.accounts.is_empty() {
        g.attach(&label("No activity in this period.", &["dim-label"]), 0, 2, 5, 1);
    }
    c.append(&g);
    c
}

fn activity_card(stats: &Stats) -> gtk::Box {
    let c = card(Some("Switches & limits"), None);
    let items: Vec<&serde_json::Value> = stats
        .events
        .iter()
        .filter(|e| matches!(e.get("kind").and_then(|k| k.as_str()), Some("switch" | "auto_switch" | "limit")))
        .take(12)
        .collect();
    if items.is_empty() {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        h.set_halign(gtk::Align::Center);
        h.set_valign(gtk::Align::Center);
        h.set_vexpand(true);
        h.append(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
        h.append(&label("No account switches or limit warnings in this period.", &["dim-label"]));
        c.append(&h);
        return c;
    }
    for e in items {
        let kind = e.get("kind").and_then(|k| k.as_str()).unwrap_or_default();
        let account = e.get("account").and_then(|a| a.as_str());
        let msg = e.get("message").and_then(|m| m.as_str()).unwrap_or_default();
        let ts = e.get("ts").and_then(|t| t.as_str()).and_then(parse_ts);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let icon = gtk::Image::from_icon_name(match kind {
            "auto_switch" => "emblem-synchronizing-symbolic",
            "switch" => "media-playlist-shuffle-symbolic",
            _ => "dialog-warning-symbolic",
        });
        icon.add_css_class("event-icon");
        if kind == "limit" {
            icon.add_css_class("limit");
        }
        icon.set_valign(gtk::Align::Start);
        row.append(&icon);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
        v.set_hexpand(true);
        let title = match kind {
            "auto_switch" => "Auto-switched".to_string(),
            "switch" => format!("Moved to {}", account.unwrap_or("another account")),
            _ => "Near limit".to_string(),
        };
        v.append(&label(&title, &["heading"]));
        let m = label(msg, &["caption", "dim-label"]);
        m.set_wrap(true);
        v.append(&m);
        row.append(&v);
        if let Some(t) = ts {
            let l = label(&model::relative(t), &["caption", "dim-label"]);
            l.set_valign(gtk::Align::Start);
            row.append(&l);
        }
        c.append(&row);
    }
    c
}
