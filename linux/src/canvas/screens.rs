//! Widget screens: review findings, row counts, documents to comment on, files and folders.

use crate::widgets::{self, label, ACCENT, RED};
use adw::prelude::*;
use claude_deck::canvas::{screens, store};
use gtk::{gio, glib};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub type OpenFn = Rc<dyn Fn(String, Option<u32>)>;
pub type ToastFn = Rc<dyn Fn(&str)>;

fn page(child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let clamp = adw::Clamp::builder().maximum_size(920).tightening_threshold(700).child(child).build();
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

pub fn resolve(cwd: &str, p: &str) -> PathBuf {
    let p = p.trim();
    if let Some(rest) = p.strip_prefix("~/") {
        return claude_deck::config::home().join(rest);
    }
    let pb = PathBuf::from(p);
    if pb.is_absolute() || cwd.is_empty() {
        pb
    } else {
        Path::new(cwd).join(pb)
    }
}

// ---------- .rv ----------

pub fn review(text: &str, dir: &Path, name: &str, on_open: OpenFn, toast: ToastFn) -> gtk::Widget {
    let rv = screens::parse_review(text);
    let cwd = store::read_meta(dir).cwd;
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.append(&label(if rv.title.is_empty() { "Review" } else { &rv.title }, &["title-2"]));
    root.append(&label(
        &format!("{} findings — keep the ones worth fixing, then send. The agent applies only those.", rv.findings.len()),
        &["dim-label"],
    ));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let checks: Rc<RefCell<Vec<(String, gtk::CheckButton)>>> = Rc::new(RefCell::new(vec![]));
    for f in &rv.findings {
        let row = gtk::ListBoxRow::new();
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        h.set_margin_top(10);
        h.set_margin_bottom(10);
        h.set_margin_start(12);
        h.set_margin_end(12);
        let keep = gtk::CheckButton::new();
        keep.set_valign(gtk::Align::Start);
        keep.set_tooltip_text(Some("Keep this finding"));
        h.append(&keep);
        let sev = gtk::Label::new(Some(&f.severity));
        sev.add_css_class("sev");
        sev.add_css_class(&format!("sev-{}", f.severity));
        sev.set_valign(gtk::Align::Start);
        h.append(&sev);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        v.set_hexpand(true);
        let m = label(&f.message, &["heading"]);
        m.set_wrap(true);
        v.append(&m);
        if !f.fix.is_empty() {
            let x = label(&f.fix, &["dim-label"]);
            x.set_wrap(true);
            v.append(&x);
        }
        if !f.file.is_empty() {
            let at = match f.line {
                Some(l) => format!("{}:{l}", f.file),
                None => f.file.clone(),
            };
            let link = gtk::Button::with_label(&at);
            link.add_css_class("link-button");
            link.add_css_class("flat");
            link.set_halign(gtk::Align::Start);
            let (o, p, l, cwd) = (on_open.clone(), f.file.clone(), f.line, cwd.clone());
            link.connect_clicked(move |_| o(resolve(&cwd, &p).to_string_lossy().to_string(), l));
            v.append(&link);
        }
        h.append(&v);
        row.set_child(Some(&h));
        let k2 = keep.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |g, _, _, _| {
            if g.widget().is_some() {
                k2.set_active(!k2.is_active());
            }
        });
        m.add_controller(click);
        list.append(&row);
        checks.borrow_mut().push((f.key.clone(), keep));
    }
    root.append(&list);
    let note = gtk::TextView::new();
    note.set_wrap_mode(gtk::WrapMode::WordChar);
    note.set_top_margin(8);
    note.set_bottom_margin(8);
    note.set_left_margin(10);
    note.set_right_margin(10);
    let nf = gtk::Frame::new(None);
    nf.set_child(Some(&note));
    nf.set_size_request(-1, 64);
    root.append(&label("Anything to add (optional)", &["caption", "dim-label"]));
    root.append(&nf);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let send = gtk::Button::with_label("Send to agent");
    send.add_css_class("suggested-action");
    let all = gtk::Button::with_label("Keep all");
    bar.append(&send);
    bar.append(&all);
    root.append(&bar);
    let c = checks.clone();
    all.connect_clicked(move |_| {
        for (_, k) in c.borrow().iter() {
            k.set_active(true);
        }
    });
    let (dir, name) = (dir.to_path_buf(), name.to_string());
    send.connect_clicked(move |_| {
        let keys: Vec<String> = checks.borrow().iter().filter(|(_, k)| k.is_active()).map(|(s, _)| s.clone()).collect();
        let b = note.buffer();
        let n = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        match store::append_event(&dir, &name, keys.clone(), n.trim()) {
            Ok(()) => toast(&format!("Sent {} kept finding{} to the agent", keys.len(), if keys.len() == 1 { "" } else { "s" })),
            Err(e) => toast(&format!("Couldn't send: {e:#}")),
        }
    });
    page(&root)
}

// ---------- .rows ----------

fn fmt_num(v: f64) -> String {
    let neg = v < 0.0;
    let s = format!("{:.0}", v.abs());
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

pub fn rows(text: &str, name: &str) -> gtk::Widget {
    let r = screens::parse_rows(text);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.append(&label(if r.title.is_empty() { name } else { &r.title }, &["title-2"]));
    let max = r.steps.iter().flat_map(|s| [s.rows_in.unwrap_or(0.0), s.rows_out.unwrap_or(0.0)]).fold(1.0, f64::max);
    for s in &r.steps {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let n = label(&s.name, &["heading"]);
        n.set_hexpand(true);
        h.append(&n);
        let counts = match (s.rows_out, s.cols) {
            (Some(o), Some(c)) => format!("{} rows · {c:.0} cols", fmt_num(o)),
            (Some(o), None) => format!("{} rows", fmt_num(o)),
            _ => "—".into(),
        };
        h.append(&label(&counts, &["numeric", "dim-label"]));
        let delta = match (s.rows_in, s.rows_out) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        };
        let d = match delta {
            Some(d) if d != 0.0 => {
                let pct = s.rows_in.filter(|a| *a != 0.0).map(|a| d / a * 100.0).unwrap_or(0.0);
                let l = label(&format!("{}{} ({:+.1}%)", if d > 0.0 { "+" } else { "" }, fmt_num(d), pct), &["numeric", "heading"]);
                l.add_css_class(if d < 0.0 { "error" } else { "success" });
                l
            }
            _ => label("unchanged", &["dim-label"]),
        };
        h.append(&d);
        v.append(&h);
        let (kept, lost, gain) = (
            s.rows_out.unwrap_or(0.0) / max,
            delta.filter(|d| *d < 0.0).map(|d| -d / max).unwrap_or(0.0),
            delta.filter(|d| *d > 0.0).map(|d| d / max).unwrap_or(0.0),
        );
        let bar = gtk::DrawingArea::new();
        bar.set_content_height(10);
        bar.set_hexpand(true);
        bar.set_draw_func(move |a, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            widgets::set(cr, widgets::fg(a), 0.08);
            widgets::rounded_rect(cr, 0.0, 0.0, w, h, 3.0);
            let _ = cr.fill();
            let k = (kept - gain).max(0.0);
            widgets::set(cr, ACCENT, 0.85);
            cr.rectangle(0.0, 0.0, w * k, h);
            let _ = cr.fill();
            widgets::set(cr, widgets::hex(0x30A46C), 0.85);
            cr.rectangle(w * k, 0.0, w * gain, h);
            let _ = cr.fill();
            widgets::set(cr, RED, 0.55);
            cr.rectangle(w * kept, 0.0, w * lost, h);
            let _ = cr.fill();
        });
        v.append(&bar);
        root.append(&v);
    }
    if r.steps.is_empty() {
        root.append(&label("No steps yet: lines look like `step | rows_in | rows_out | columns`.", &["dim-label"]));
    }
    page(&root)
}

// ---------- Markdown ----------

fn inline_md(s: &str) -> String {
    let e = glib::markup_escape_text(s).to_string();
    let mut out = String::new();
    let mut rest = e.as_str();
    // `code`, **bold**, *italic*, [text](url)
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('`') {
            if let Some(end) = r.find('`') {
                out.push_str(&format!("<tt>{}</tt>", &r[..end]));
                rest = &r[end + 1..];
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix("**") {
            if let Some(end) = r.find("**") {
                out.push_str(&format!("<b>{}</b>", &r[..end]));
                rest = &r[end + 2..];
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix('*') {
            if let Some(end) = r.find('*') {
                if end > 0 && !r[..end].starts_with(' ') {
                    out.push_str(&format!("<i>{}</i>", &r[..end]));
                    rest = &r[end + 1..];
                    continue;
                }
            }
        }
        if let Some(r) = rest.strip_prefix('[') {
            if let Some(mid) = r.find("](") {
                if let Some(end) = r[mid + 2..].find(')') {
                    let (t, u) = (&r[..mid], &r[mid + 2..mid + 2 + end]);
                    out.push_str(&format!("<a href=\"{u}\">{t}</a>"));
                    rest = &r[mid + 3 + end..];
                    continue;
                }
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// One Markdown block as a widget.
pub fn md_block(b: &str) -> gtk::Widget {
    let t = b.trim();
    if t.starts_with("```") {
        let code: Vec<&str> = t.lines().skip(1).filter(|l| !l.trim_start().starts_with("```")).collect();
        let l = gtk::Label::new(Some(&code.join("\n")));
        l.set_xalign(0.0);
        l.set_selectable(true);
        l.add_css_class("monospace");
        l.add_css_class("code-block");
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::Char);
        return l.upcast();
    }
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_selectable(false);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    let hashes = t.chars().take_while(|&c| c == '#').count();
    let markup = if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') && !t.contains('\n') {
        let size = ["xx-large", "x-large", "large", "medium", "medium", "medium"][hashes - 1];
        format!("<span size=\"{size}\" weight=\"bold\">{}</span>", inline_md(t[hashes..].trim()))
    } else if t.lines().all(|l| l.trim_start().starts_with("- ") || l.trim_start().starts_with("* ") || l.trim_start().starts_with("+ ")) {
        t.lines()
            .map(|l| {
                let indent = l.len() - l.trim_start().len();
                format!("{}•  {}", " ".repeat(indent), inline_md(&l.trim_start()[2..]))
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else if t.lines().all(|l| l.trim_start().starts_with('>')) {
        format!("<i>{}</i>", inline_md(&t.lines().map(|l| l.trim_start().trim_start_matches('>').trim()).collect::<Vec<_>>().join(" ")))
    } else if t.lines().all(|l| l.trim_start().starts_with('|')) {
        l.add_css_class("monospace");
        glib::markup_escape_text(t).to_string()
    } else {
        inline_md(&t.lines().collect::<Vec<_>>().join("\n"))
    };
    l.set_markup(&markup);
    l.upcast()
}

pub fn markdown(text: &str) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("doc");
    for b in screens::blocks(text) {
        root.append(&md_block(&b));
    }
    page(&root)
}

// ---------- .doc ----------

pub fn doc(spec: &str, dir: &Path, name: &str, toast: ToastFn) -> gtk::Widget {
    let cwd = store::read_meta(dir).cwd;
    let (p, _) = screens::parse_path(spec);
    let path = resolve(&cwd, &p);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return error_page(&format!("Couldn't read {}: {e}", path.display())),
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.append(&label(&crate::model::short_path(&path.to_string_lossy()), &["dim-label", "caption"]));
    root.append(&label("Click a paragraph to comment on it; send the comments to the agent when done.", &["dim-label"]));
    let list = gtk::ListBox::new();
    list.add_css_class("doc-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let comments: Rc<RefCell<Vec<(usize, String, String)>>> = Rc::new(RefCell::new(vec![]));
    let summary = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let send = gtk::Button::with_label("Send comments to agent");
    send.add_css_class("suggested-action");
    send.set_halign(gtk::Align::Start);
    send.set_visible(false);
    let redraw = {
        let (summary, send, comments) = (summary.clone(), send.clone(), comments.clone());
        Rc::new(move || {
            while let Some(c) = summary.first_child() {
                summary.remove(&c);
            }
            for (p, excerpt, t) in comments.borrow().iter() {
                let l = gtk::Label::new(None);
                l.set_xalign(0.0);
                l.set_wrap(true);
                l.set_markup(&format!(
                    "<b>¶{p}</b> {} <span alpha=\"60%\">— {}…</span>",
                    glib::markup_escape_text(t),
                    glib::markup_escape_text(excerpt)
                ));
                summary.append(&l);
            }
            send.set_visible(!comments.borrow().is_empty());
        })
    };
    for (i, b) in screens::blocks(&text).into_iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        v.set_margin_top(4);
        v.set_margin_bottom(4);
        v.append(&md_block(&b));
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Comment on this paragraph — Enter to add, Esc to cancel"));
        entry.set_visible(false);
        v.append(&entry);
        row.set_child(Some(&v));
        let e2 = entry.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            if !WidgetExt::is_visible(&e2) {
                e2.set_visible(true);
                e2.grab_focus();
            }
        });
        v.add_controller(click);
        let (comments, redraw, row2) = (comments.clone(), redraw.clone(), row.clone());
        let excerpt: String = b.chars().filter(|c| *c != '#').take(60).collect::<String>().trim().replace('\n', " ");
        entry.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            e.set_text("");
            e.set_visible(false);
            if !t.is_empty() {
                comments.borrow_mut().push((i + 1, excerpt.clone(), t));
                row2.add_css_class("commented");
                redraw();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let e3 = entry.clone();
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape {
                e3.set_text("");
                e3.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(keys);
        list.append(&row);
    }
    root.append(&list);
    root.append(&summary);
    root.append(&send);
    let (dir, name) = (dir.to_path_buf(), name.to_string());
    send.connect_clicked(move |_| {
        let c = comments.borrow();
        let choices: Vec<String> = c.iter().map(|(p, _, _)| format!("¶{p}")).collect();
        let note = c.iter().map(|(p, ex, t)| format!("¶{p} ({ex}…): {t}")).collect::<Vec<_>>().join("\n");
        match store::append_event(&dir, &name, choices, &note) {
            Ok(()) => toast("Comments sent to the agent"),
            Err(e) => toast(&format!("Couldn't send: {e:#}")),
        }
    });
    page(&root)
}

// ---------- files ----------

pub fn error_page(msg: &str) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("Can't show this screen")
        .description(glib::markup_escape_text(msg).as_str())
        .build()
        .upcast()
}

/// A file (scrolled to `line`) or a folder listing; folders navigate in place.
pub fn file(path: &Path, line: Option<u32>, on_open: OpenFn) -> gtk::Widget {
    let md = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return error_page(&format!("{}: {e}", path.display())),
    };
    if md.is_dir() {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.append(&label(&crate::model::short_path(&path.to_string_lossy()), &["title-3"]));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        let mut entries: Vec<(bool, String)> = std::fs::read_dir(path)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        (!n.starts_with('.')).then(|| (e.path().is_dir(), n))
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if let Some(parent) = path.parent() {
            entries.insert(0, (true, "..".into()));
            let _ = parent;
        }
        for (is_dir, n) in entries {
            let row = adw::ActionRow::builder().title(glib::markup_escape_text(&n).as_str()).activatable(true).build();
            row.add_prefix(&gtk::Image::from_icon_name(if is_dir { "folder-symbolic" } else { "text-x-generic-symbolic" }));
            let target = if n == ".." { path.parent().unwrap_or(path).to_path_buf() } else { path.join(&n) };
            let o = on_open.clone();
            row.connect_activated(move |_| o(target.to_string_lossy().to_string(), None));
            list.append(&row);
        }
        root.append(&list);
        return page(&root);
    }
    if md.len() > 4 * 1024 * 1024 {
        return error_page(&format!("{} is larger than 4 MB", path.display()));
    }
    let bytes = std::fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let buf = gtk::TextBuffer::new(None);
    buf.set_text(&text);
    let view = gtk::TextView::with_buffer(&buf);
    view.set_editable(false);
    view.set_monospace(true);
    view.set_left_margin(16);
    view.set_top_margin(12);
    view.set_bottom_margin(24);
    view.set_wrap_mode(gtk::WrapMode::None);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&view));
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.set_margin_start(16);
    head.set_margin_end(10);
    head.set_margin_top(6);
    head.set_margin_bottom(6);
    let title = label(&crate::model::short_path(&path.to_string_lossy()), &["dim-label", "caption"]);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    head.append(&title);
    let ext = gtk::Button::from_icon_name("document-open-symbolic");
    ext.add_css_class("flat");
    ext.set_tooltip_text(Some("Open with the default app"));
    let file = gio::File::for_path(path);
    ext.connect_clicked(move |b| {
        let win = b.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        gtk::FileLauncher::new(Some(&file)).launch(win.as_ref(), None::<&gio::Cancellable>, |_| {});
    });
    head.append(&ext);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&head);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&scroll);
    if let Some(l) = line.filter(|l| *l > 0) {
        let tag = buf.create_tag(Some("hl"), &[("paragraph-background", &"rgba(217,119,87,0.22)")]);
        if let (Some(tag), Some(start)) = (tag, buf.iter_at_line(l as i32 - 1)) {
            let mut end = start;
            end.forward_to_line_end();
            buf.apply_tag(&tag, &start, &end);
            let v = view.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                let mut it = v.buffer().iter_at_line(l as i32 - 1).unwrap_or(v.buffer().start_iter());
                v.scroll_to_iter(&mut it, 0.0, true, 0.0, 0.3);
            });
        }
    }
    root.upcast()
}

/// Mermaid types without a native renderer, HTML and plans: the source, plus a way out.
pub fn source(text: &str, note: &str, external: Option<PathBuf>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let n = label(note, &["dim-label"]);
    n.set_wrap(true);
    root.append(&n);
    if let Some(p) = external {
        let b = gtk::Button::with_label("Open in browser");
        b.set_halign(gtk::Align::Start);
        b.add_css_class("pill");
        let file = gio::File::for_path(&p);
        b.connect_clicked(move |b| {
            let win = b.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            gtk::FileLauncher::new(Some(&file)).launch(win.as_ref(), None::<&gio::Cancellable>, |_| {});
        });
        root.append(&b);
    }
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_selectable(true);
    l.add_css_class("monospace");
    l.add_css_class("code-block");
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::Char);
    root.append(&l);
    page(&root)
}
