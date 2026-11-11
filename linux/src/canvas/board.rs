//! Board: long-lived objectives with their tasks, and what every agent session is doing.
//! Edits rewrite the objective file and reach the sessions working on it with their next prompt.

use super::{state_color, state_text};
use crate::widgets::{self, label};
use adw::prelude::*;
use claude_deck::canvas::objectives::{self, Objective, Op, Task};
use claude_deck::canvas::store::Session;
use gtk::glib;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub struct Board {
    pub root: gtk::ScrolledWindow,
    content: gtk::Box,
    sessions: RefCell<Vec<Session>>,
    pub on_toast: super::Callback<dyn Fn(&str)>,
    /// Go to the session: its terminal tab if it runs in CC Deck, else its canvas.
    pub on_goto: super::Callback<dyn Fn(&Session)>,
    me: RefCell<Weak<Board>>,
}

fn glyph(state: &str) -> &'static str {
    match state {
        "doing" => "◐",
        "done" => "●",
        "blocked" => "⊘",
        _ => "○",
    }
}

fn next_state(state: &str) -> &'static str {
    match state {
        "todo" => "doing",
        "doing" => "done",
        _ => "todo",
    }
}

impl Board {
    pub fn new() -> Rc<Board> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 22);
        content.set_margin_top(22);
        content.set_margin_bottom(28);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let clamp = adw::Clamp::builder().maximum_size(980).tightening_threshold(760).child(&content).build();
        let root = gtk::ScrolledWindow::new();
        root.set_hscrollbar_policy(gtk::PolicyType::Never);
        root.set_child(Some(&clamp));
        let b = Rc::new(Board {
            root,
            content,
            sessions: RefCell::new(vec![]),
            on_toast: RefCell::new(None),
            on_goto: RefCell::new(None),
            me: RefCell::new(Weak::new()),
        });
        *b.me.borrow_mut() = Rc::downgrade(&b);
        b.render();
        b
    }

    fn toast(&self, s: &str) {
        if let Some(f) = self.on_toast.borrow().clone() {
            f(s);
        }
    }

    pub fn update(&self, sessions: Vec<Session>) {
        *self.sessions.borrow_mut() = sessions;
        self.render();
    }

    fn apply(&self, slug: &str, op: Op) {
        match objectives::apply(slug, &op) {
            Ok(said) => self.toast(&format!("{said} — agents on it hear this with their next prompt")),
            Err(e) => self.toast(&format!("{e:#}")),
        }
        self.render();
    }

    fn session_chip(&self, s: &Session) -> gtk::Widget {
        let b = gtk::Button::new();
        b.add_css_class("flat");
        b.add_css_class("session-chip");
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dot = widgets::dot(state_color(&s.state.state).unwrap_or(widgets::hex(0x8B8D98)), 8);
        h.append(&dot);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let t = label(&s.label(), &[]);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(48);
        v.append(&t);
        let mut sub = vec![s.meta.agent.clone(), s.meta.project.clone()];
        if !s.meta.account.is_empty() {
            sub.push(s.meta.account.clone());
        }
        let st = state_text(s);
        if !st.is_empty() {
            sub.push(st);
        }
        let sl = label(&sub.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "), &["caption", "dim-label"]);
        sl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sl.set_max_width_chars(60);
        if s.needs_you() {
            sl.remove_css_class("dim-label");
            sl.add_css_class("warning");
        }
        v.append(&sl);
        h.append(&v);
        b.set_child(Some(&h));
        b.set_tooltip_text(Some("Go to this session"));
        let me = self.me.borrow().clone();
        let s2 = s.clone();
        b.connect_clicked(move |_| {
            if let Some(f) = me.upgrade().and_then(|m| m.on_goto.borrow().clone()) {
                f(&s2);
            }
        });
        b.upcast()
    }

    fn task_row(&self, o: &Objective, t: &Task) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let state_btn = gtk::Button::with_label(glyph(&t.state));
        state_btn.add_css_class("flat");
        state_btn.add_css_class("task-state");
        state_btn.add_css_class(&format!("task-{}", t.state));
        state_btn.set_tooltip_text(Some(&format!("{} — click for {}", t.state, next_state(&t.state))));
        let me = self.me.borrow().clone();
        let (slug, line, text, st) = (o.slug.clone(), t.line, t.text.clone(), t.state.clone());
        state_btn.connect_clicked(move |_| {
            if let Some(b) = me.upgrade() {
                b.apply(&slug, Op::State { line, text: text.clone(), state: next_state(&st).into() });
            }
        });
        h.append(&state_btn);
        let tl = label(&t.text, &[]);
        tl.set_wrap(true);
        tl.set_hexpand(true);
        if t.state == "done" {
            tl.add_css_class("dim-label");
        }
        h.append(&tl);

        let more = gtk::MenuButton::new();
        more.set_icon_name("view-more-symbolic");
        more.add_css_class("flat");
        more.set_valign(gtk::Align::Center);
        let pop = gtk::Popover::new();
        let pb = gtk::Box::new(gtk::Orientation::Vertical, 6);
        pb.set_margin_top(8);
        pb.set_margin_bottom(8);
        pb.set_margin_start(8);
        pb.set_margin_end(8);
        for st in ["todo", "doing", "done", "blocked"] {
            let b = gtk::Button::with_label(&format!("{}  {st}", glyph(st)));
            b.add_css_class("flat");
            b.set_sensitive(st != t.state);
            let me = self.me.borrow().clone();
            let (slug, line, text, p) = (o.slug.clone(), t.line, t.text.clone(), pop.clone());
            b.connect_clicked(move |_| {
                p.popdown();
                if let Some(bd) = me.upgrade() {
                    bd.apply(&slug, Op::State { line, text: text.clone(), state: st.into() });
                }
            });
            pb.append(&b);
        }
        let note = gtk::Entry::new();
        note.set_placeholder_text(Some("Add a note — Enter"));
        let me = self.me.borrow().clone();
        let (slug, line, text, p) = (o.slug.clone(), t.line, t.text.clone(), pop.clone());
        note.connect_activate(move |e| {
            let n = e.text().to_string();
            p.popdown();
            if let Some(b) = me.upgrade() {
                b.apply(&slug, Op::Note { line, text: text.clone(), note: n });
            }
        });
        pb.append(&note);
        pop.set_child(Some(&pb));
        more.set_popover(Some(&pop));
        h.append(&more);
        row.append(&h);
        for n in &t.notes {
            let nl = label(&format!("› {n}"), &["caption", "dim-label"]);
            nl.set_wrap(true);
            nl.set_margin_start(40);
            row.append(&nl);
        }
        row.upcast()
    }

    fn objective_card(&self, o: &Objective, sessions: &[Session]) -> gtk::Widget {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        card.add_css_class("card");
        card.add_css_class("deck-card");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let tv = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tv.set_hexpand(true);
        let t = label(&o.title, &["title-4"]);
        t.set_wrap(true);
        tv.append(&t);
        let blocked = o.tasks.iter().filter(|t| t.state == "blocked").count();
        let mut sub = format!("{}/{} done · {}", o.done(), o.tasks.len(), o.slug);
        if blocked > 0 {
            sub = format!("{sub} · {blocked} blocked");
        }
        tv.append(&label(&sub, &["caption", "dim-label", "numeric"]));
        head.append(&tv);
        let status = gtk::DropDown::from_strings(&["active", "paused", "done"]);
        status.set_selected(match o.status.as_str() {
            "paused" => 1,
            "done" => 2,
            _ => 0,
        });
        status.set_valign(gtk::Align::Start);
        let me = self.me.borrow().clone();
        let slug = o.slug.clone();
        status.connect_selected_notify(move |d| {
            let s = ["active", "paused", "done"][d.selected() as usize % 3];
            let (me, slug) = (me.clone(), slug.clone());
            glib::idle_add_local_once(move || {
                if let Some(b) = me.upgrade() {
                    b.apply(&slug, Op::Status { status: s.into() });
                }
            });
        });
        head.append(&status);
        card.append(&head);
        let frac = if o.tasks.is_empty() { 0.0 } else { o.done() as f64 / o.tasks.len() as f64 };
        card.append(&widgets::thin_bar(frac * 100.0, widgets::ACCENT, 5));
        for t in &o.tasks {
            card.append(&self.task_row(o, t));
        }
        let add = gtk::Entry::new();
        add.set_placeholder_text(Some("Add a task — Enter"));
        let me = self.me.borrow().clone();
        let slug = o.slug.clone();
        add.connect_activate(move |e| {
            let t = e.text().to_string();
            if t.trim().is_empty() {
                return;
            }
            if let Some(b) = me.upgrade() {
                b.apply(&slug, Op::Add { text: t });
            }
        });
        card.append(&add);
        let on: Vec<&Session> = sessions.iter().filter(|s| s.meta.objective == o.slug && s.state.state != "ended").collect();
        if !on.is_empty() {
            card.append(&label("Sessions on it", &["caption-heading", "dim-label"]));
            let flow = gtk::FlowBox::new();
            flow.set_selection_mode(gtk::SelectionMode::None);
            flow.set_max_children_per_line(3);
            flow.set_column_spacing(6);
            for s in on {
                flow.insert(&self.session_chip(s), -1);
            }
            card.append(&flow);
        }
        card.upcast()
    }

    fn render(&self) {
        while let Some(c) = self.content.first_child() {
            self.content.remove(&c);
        }
        let sessions = self.sessions.borrow().clone();
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&label("Board", &["dash-title"]));
        head.append(&label(
            "Objectives your agents work on, and what each session is doing. Ticking or noting a task reaches the agents on it with their next prompt.",
            &["dim-label"],
        ));
        self.content.append(&head);

        let waiting: Vec<&Session> = sessions.iter().filter(|s| s.needs_you()).collect();
        if !waiting.is_empty() {
            let sec = gtk::Box::new(gtk::Orientation::Vertical, 8);
            sec.append(&label(&format!("Needs you ({})", waiting.len()), &["heading", "warning"]));
            for s in waiting {
                sec.append(&self.session_chip(s));
            }
            self.content.append(&sec);
        }

        let objs = objectives::list();
        let (active, rest): (Vec<&Objective>, Vec<&Objective>) = objs.iter().partition(|o| o.status != "done");
        if objs.is_empty() {
            let e = label(
                "No objectives yet. An agent starts one with `claude-deck canvas attach <session dir> <slug> \"Title\"`, or create the file ~/.local/share/claude-deck/canvas/objectives/<slug>.md yourself.",
                &["dim-label"],
            );
            e.set_wrap(true);
            self.content.append(&e);
        }
        for o in active {
            self.content.append(&self.objective_card(o, &sessions));
        }

        let live: Vec<&Session> = sessions
            .iter()
            .filter(|s| s.meta.objective.is_empty() && !s.state.state.is_empty() && s.state.state != "ended" && !s.needs_you())
            .collect();
        if !live.is_empty() {
            let sec = gtk::Box::new(gtk::Orientation::Vertical, 8);
            sec.append(&label("Other sessions", &["heading"]));
            let flow = gtk::FlowBox::new();
            flow.set_selection_mode(gtk::SelectionMode::None);
            flow.set_max_children_per_line(2);
            flow.set_column_spacing(6);
            for s in live {
                flow.insert(&self.session_chip(s), -1);
            }
            sec.append(&flow);
            self.content.append(&sec);
        }
        if !rest.is_empty() {
            let exp = gtk::Expander::new(Some(&format!("Done ({})", rest.len())));
            let v = gtk::Box::new(gtk::Orientation::Vertical, 12);
            v.set_margin_top(10);
            for o in rest {
                v.append(&self.objective_card(o, &sessions));
            }
            exp.set_child(Some(&v));
            self.content.append(&exp);
        }
    }
}
