//! `.plan` screens: the work breakdown with live Jira status, comments for the agent, the
//! dependency graph, and Push to Jira.

use super::screens::ToastFn;
use crate::bg;
use crate::widgets::label;
use adw::prelude::*;
use claude_deck::canvas::{jira, plan, store};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn pill(text: &str, category: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("status-pill");
    l.add_css_class(match category {
        "done" => "cat-done",
        "indeterminate" => "cat-doing",
        _ => "cat-todo",
    });
    l.set_valign(gtk::Align::Center);
    l
}

fn key_widget(key: &str, live: &HashMap<String, Value>, base: &str) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if key.is_empty() {
        b.append(&pill("new", "todo"));
        return b.upcast();
    }
    let link = gtk::LinkButton::with_label(&format!("{base}/browse/{key}"), key);
    link.add_css_class("flat");
    b.append(&link);
    if let Some(l) = live.get(key) {
        b.append(&pill(l["status"].as_str().unwrap_or(""), l["category"].as_str().unwrap_or("")));
    }
    b.upcast()
}

pub fn plan_view(text: &str, file: PathBuf, dir: &Path, toast: ToastFn) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.set_vexpand(true);
    root.append(&holder);
    let p = plan::parse(text);
    let render: Rc<RefCell<Option<Box<dyn Fn(&HashMap<String, Value>)>>>> = Rc::new(RefCell::new(None));
    let comments: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(vec![]));
    let (h2, file2, dir2, toast2, comments2) = (holder.clone(), file.clone(), dir.to_path_buf(), toast.clone(), comments.clone());
    let p2 = p.clone();
    *render.borrow_mut() = Some(Box::new(move |live: &HashMap<String, Value>| {
        while let Some(c) = h2.first_child() {
            h2.remove(&c);
        }
        h2.append(&build(&p2, live, &file2, &dir2, toast2.clone(), comments2.clone()));
    }));
    if let Some(f) = render.borrow().as_ref() {
        f(&HashMap::new());
    }
    let keys = p.keys();
    if !keys.is_empty() && jira::configured() {
        let r = render.clone();
        bg::run(move || jira::statuses(&keys), move |res| {
            if let (Ok(live), Some(f)) = (res, r.borrow().as_ref()) {
                f(&live);
            }
        });
    }
    root.upcast()
}

fn build(p: &plan::Plan, live: &HashMap<String, Value>, file: &Path, dir: &Path, toast: ToastFn, comments: Rc<RefCell<Vec<(String, String)>>>) -> gtk::Widget {
    let base = jira::base();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let tv = gtk::Box::new(gtk::Orientation::Vertical, 2);
    tv.set_hexpand(true);
    let title = if p.epic.summary.is_empty() { p.epic.key.clone() } else { p.epic.summary.clone() };
    tv.append(&label(&title, &["title-2"]));
    tv.append(&label(
        &format!("{} · {} items · {} pt{}", if p.project.is_empty() { "no project" } else { &p.project }, p.items.len(), p.total, if p.labels.is_empty() { String::new() } else { format!(" · labels {}", p.labels.join(", ")) }),
        &["dim-label"],
    ));
    head.append(&tv);
    head.append(&key_widget(&p.epic.key, live, &base));
    root.append(&head);
    if !p.epic.description.is_empty() {
        let d = label(&p.epic.description.join("\n"), &[]);
        d.set_wrap(true);
        root.append(&d);
    }
    for e in &p.errors {
        let l = label(&format!("⚠ {e}"), &["error"]);
        l.set_wrap(true);
        root.append(&l);
    }

    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for it in &p.items {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("card");
        card.add_css_class("deck-card");
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        h.append(&label(&format!("#{}", it.n), &["dim-label", "numeric"]));
        let ty = gtk::Label::new(Some(&it.kind));
        ty.add_css_class("type-badge");
        ty.add_css_class(&format!("type-{}", it.kind.to_lowercase()));
        h.append(&ty);
        let s = label(&it.f.summary, &["heading"]);
        s.set_wrap(true);
        s.set_width_chars(12);
        s.set_hexpand(true);
        h.append(&s);
        let mut meta = vec![];
        if let Some(pt) = it.f.points {
            meta.push(format!("{pt} pt"));
        }
        if it.total != it.f.points.unwrap_or(0.0) {
            meta.push(format!("Σ {} pt", it.total));
        }
        if !it.f.assignee.is_empty() {
            meta.push(format!("@{}", it.f.assignee));
        }
        if !it.f.labels.is_empty() {
            meta.push(it.f.labels.join(", "));
        }
        let ml = label(&meta.join(" · "), &["dim-label", "caption"]);
        ml.set_ellipsize(gtk::pango::EllipsizeMode::End);
        ml.set_max_width_chars(28);
        h.append(&ml);
        h.append(&key_widget(&it.f.key, live, &base));
        card.append(&h);
        if !it.description.is_empty() {
            let d = label(&it.description.join("\n"), &["dim-label"]);
            d.set_wrap(true);
            card.append(&d);
        }
        if !it.ac.is_empty() {
            card.append(&label("Acceptance criteria", &["caption-heading"]));
            for a in &it.ac {
                let l = label(&format!("•  {a}"), &[]);
                l.set_wrap(true);
                card.append(&l);
            }
        }
        for st in &it.subtasks {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            r.set_margin_start(18);
            r.append(&label("↳", &["dim-label"]));
            let l = label(&st.f.summary, &[]);
            l.set_hexpand(true);
            l.set_wrap(true);
            r.append(&l);
            if let Some(pt) = st.f.points {
                r.append(&label(&format!("{pt} pt"), &["dim-label", "caption"]));
            }
            r.append(&key_widget(&st.f.key, live, &base));
            card.append(&r);
        }
        if !it.after.is_empty() {
            card.append(&label(&format!("after {}", it.after.join(", ")), &["caption", "dim-label"]));
        }
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Comment for the agent — Enter"));
        let (c2, tag) = (comments.clone(), format!("#{} {}", it.n, it.f.summary));
        entry.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if !t.is_empty() {
                c2.borrow_mut().push((tag.clone(), t));
                e.set_text("");
                e.set_placeholder_text(Some(&format!("{} comment(s) added — send them below", c2.borrow().len())));
            }
        });
        card.append(&entry);
        list.append(&card);
    }
    root.append(&list);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let send = gtk::Button::with_label("Send comments to agent");
    let graph = gtk::Button::with_label("Dependency graph");
    let push = gtk::Button::with_label("Push to Jira…");
    push.add_css_class("suggested-action");
    push.set_sensitive(p.errors.is_empty() && jira::configured());
    if !jira::configured() {
        push.set_tooltip_text(Some(&format!("Set JIRA_BASE_URL, JIRA_EMAIL and JIRA_API_TOKEN in {}", jira::env_file().display())));
    }
    bar.append(&push);
    bar.append(&graph);
    bar.append(&send);
    root.append(&bar);

    let (dir2, name, t2) = (dir.to_path_buf(), file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), toast.clone());
    send.connect_clicked(move |_| {
        let c = comments.borrow();
        if c.is_empty() {
            t2("Add a comment on an item first");
            return;
        }
        let note = c.iter().map(|(k, t)| format!("{k}: {t}")).collect::<Vec<_>>().join("\n");
        match store::append_event(&dir2, &name, c.iter().map(|(k, _)| k.clone()).collect(), &note) {
            Ok(()) => t2("Comments sent to the agent; it revises the plan with your next prompt"),
            Err(e) => t2(&format!("{e:#}")),
        }
    });
    let (f2, p2, t3) = (file.to_path_buf(), p.clone(), toast.clone());
    graph.connect_clicked(move |_| {
        let (f3, p3, t4) = (f2.clone(), p2.clone(), t3.clone());
        bg::run(
            move || {
                let keys: Vec<String> = p3.items.iter().map(|i| i.f.key.clone()).filter(|k| !k.is_empty()).collect();
                let live = if keys.is_empty() || !jira::configured() { Default::default() } else { jira::statuses(&keys).unwrap_or_default() };
                let out = f3.with_file_name(format!("{}-deps.mmd", f3.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
                std::fs::write(&out, plan::graph(&p3, &live, &jira::base()))?;
                Ok(out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
            },
            move |r| match r {
                Ok(n) => t4(&format!("Wrote {n}; it opens on the canvas")),
                Err(e) => t4(&e),
            },
        );
    });
    let (f4, d4, t5) = (file.to_path_buf(), dir.to_path_buf(), toast);
    let new_items = p.items.iter().filter(|i| i.f.key.is_empty()).count() + p.items.iter().map(|i| i.subtasks.iter().filter(|s| s.f.key.is_empty()).count()).sum::<usize>() + usize::from(p.epic.key.is_empty());
    let project = p.project.clone();
    push.connect_clicked(move |b| {
        let alert = adw::AlertDialog::new(
            Some("Push this plan to Jira?"),
            Some(&format!("Creates {new_items} new issue(s) in {project} and links the `after:` dependencies. Issues that already have a key are left as they are.")),
        );
        alert.add_response("cancel", "Cancel");
        alert.add_response("push", "Push");
        alert.set_response_appearance("push", adw::ResponseAppearance::Suggested);
        alert.set_close_response("cancel");
        let (f5, d5, t6) = (f4.clone(), d4.clone(), t5.clone());
        alert.connect_response(None, move |_, r| {
            if r != "push" {
                return;
            }
            let (f6, d6, t7) = (f5.clone(), d5.clone(), t6.clone());
            let name = f6.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            t7("Pushing to Jira…");
            bg::run(
                move || plan::push(&f6),
                move |res| match res {
                    Ok(r) => {
                        let mut all = r.created.clone();
                        all.extend(r.linked.clone());
                        if !all.is_empty() {
                            // the agent that drafted the plan learns the new keys with its next prompt
                            let _ = store::append_event(&d6, &name, vec![format!("pushed to Jira: {}", all.join("; "))], "");
                        }
                        t7(&format!("Created {} issue(s), {} link(s) under {}", r.created.len(), r.linked.len(), r.epic));
                    }
                    Err(e) => t7(&e),
                },
            );
        });
        alert.present(Some(b));
    });
    let clamp = adw::Clamp::builder().maximum_size(920).child(&root).build();
    clamp.set_margin_top(20);
    clamp.set_margin_bottom(28);
    clamp.set_margin_start(24);
    clamp.set_margin_end(24);
    let s = gtk::ScrolledWindow::new();
    s.set_hscrollbar_policy(gtk::PolicyType::Never);
    s.set_vexpand(true);
    s.set_child(Some(&clamp));
    s.upcast()
}
