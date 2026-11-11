//! Git changes of a project: the changed files and a coloured diff of the selected one.

use crate::bg;
use crate::widgets::label;
use adw::prelude::*;
use claude_deck::canvas::git;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn diff_buffer(text: &str) -> gtk::TextBuffer {
    let buf = gtk::TextBuffer::new(None);
    let tags = buf.tag_table();
    let mk = |name: &str, props: &[(&str, &dyn ToValue)]| {
        let t = gtk::TextTag::new(Some(name));
        for (k, v) in props {
            t.set_property_from_value(k, &v.to_value());
        }
        tags.add(&t);
    };
    mk("add", &[("paragraph-background", &"rgba(48,164,108,0.16)"), ("foreground", &"#3aa86e")]);
    mk("del", &[("paragraph-background", &"rgba(229,72,77,0.15)"), ("foreground", &"#d9534f")]);
    mk("hunk", &[("foreground", &"#4c9ae8"), ("weight", &700i32)]);
    mk("meta", &[("foreground", &"#8b8d98")]);
    for line in text.lines() {
        let tag = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") || line.starts_with("new file") {
            "meta"
        } else if line.starts_with("@@") {
            "hunk"
        } else if line.starts_with('+') {
            "add"
        } else if line.starts_with('-') {
            "del"
        } else {
            ""
        };
        let mut end = buf.end_iter();
        if tag.is_empty() {
            buf.insert(&mut end, &format!("{line}\n"));
        } else {
            buf.insert_with_tags_by_name(&mut end, &format!("{line}\n"), &[tag]);
        }
    }
    buf
}

pub fn changes(dir: PathBuf) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.set_margin_start(14);
    head.set_margin_end(10);
    head.set_margin_top(8);
    head.set_margin_bottom(8);
    let title = label(&crate::model::short_path(&dir.to_string_lossy()), &["heading"]);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    head.append(&title);
    let summary = label("", &["dim-label", "numeric"]);
    head.append(&summary);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.add_css_class("flat");
    refresh.set_tooltip_text(Some("Refresh"));
    head.append(&refresh);
    root.append(&head);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let files = gtk::ListBox::new();
    files.add_css_class("navigation-sidebar");
    let fs = gtk::ScrolledWindow::new();
    fs.set_hscrollbar_policy(gtk::PolicyType::Never);
    fs.set_child(Some(&files));
    fs.set_size_request(260, -1);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_left_margin(12);
    view.set_top_margin(8);
    let vs = gtk::ScrolledWindow::new();
    vs.set_child(Some(&view));
    vs.set_hexpand(true);
    let pane = gtk::Paned::new(gtk::Orientation::Horizontal);
    pane.set_start_child(Some(&fs));
    pane.set_end_child(Some(&vs));
    pane.set_position(300);
    pane.set_vexpand(true);
    root.append(&pane);

    let state: Rc<RefCell<Vec<git::File>>> = Rc::new(RefCell::new(vec![]));
    let load: Rc<dyn Fn()> = {
        let (files, summary, title, view, state, dir) = (files.clone(), summary.clone(), title.clone(), view.clone(), state.clone(), dir.clone());
        Rc::new(move || {
            let d = dir.clone();
            let (files, summary, title, view, state) = (files.clone(), summary.clone(), title.clone(), view.clone(), state.clone());
            bg::run(move || Ok(git::status(&d)), move |r| {
                files.remove_all();
                match r {
                    Ok(Some(s)) => {
                        title.set_text(&format!("{} · {}", crate::model::short_path(&s.root), s.branch));
                        summary.set_markup(&format!("{} files  <span foreground=\"#3aa86e\">+{}</span>  <span foreground=\"#d9534f\">−{}</span>", s.files.len(), s.added, s.deleted));
                        for f in &s.files {
                            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            let l = gtk::Label::new(Some(&f.letter));
                            l.add_css_class("git-letter");
                            l.add_css_class(&format!("git-{}", f.status));
                            row.append(&l);
                            let p = label(&f.path, &[]);
                            p.set_ellipsize(gtk::pango::EllipsizeMode::Start);
                            p.set_hexpand(true);
                            p.set_tooltip_text(Some(&format!("{} {}", f.status, f.path)));
                            row.append(&p);
                            let n = gtk::Label::new(None);
                            n.set_markup(&format!("<span foreground=\"#3aa86e\">+{}</span> <span foreground=\"#d9534f\">−{}</span>", f.added, f.deleted));
                            n.add_css_class("caption");
                            row.append(&n);
                            files.append(&row);
                        }
                        if s.files.is_empty() {
                            view.set_buffer(Some(&diff_buffer("Working tree clean.")));
                        }
                        *state.borrow_mut() = s.files;
                    }
                    Ok(None) => {
                        summary.set_text("not a git repository");
                        *state.borrow_mut() = vec![];
                    }
                    Err(e) => summary.set_text(&e),
                }
            });
        })
    };
    let (view2, state2, dir2) = (view.clone(), state.clone(), dir.clone());
    files.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Some(f) = state2.borrow().get(row.index() as usize).cloned() else { return };
        let (d, v) = (dir2.clone(), view2.clone());
        bg::run(move || Ok(git::diff(&d, &f.path, f.untracked)), move |r| {
            v.set_buffer(Some(&diff_buffer(&r.unwrap_or_else(|e| e))));
        });
    });
    let l2 = load.clone();
    refresh.connect_clicked(move |_| l2());
    load();
    root.upcast()
}
