//! Files section of the explorer: a lazily expanded tree of a folder (the active terminal's).

use crate::canvas::Callback;
use crate::widgets::label;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

const MAX_ENTRIES: usize = 2000;
const HIDDEN: [&str; 3] = [".git", ".DS_Store", "__pycache__"];

pub fn icon_for(path: &Path) -> &'static str {
    if path.is_dir() {
        return "folder-symbolic";
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" => "image-x-generic-symbolic",
        "md" | "txt" | "rst" | "pdf" | "doc" | "docx" => "x-office-document-symbolic",
        "csv" | "tsv" | "xlsx" | "parquet" => "x-office-spreadsheet-symbolic",
        "sh" | "bash" | "zsh" | "py" | "rs" | "js" | "ts" | "tsx" | "jsx" | "go" | "swift" | "sql" | "rb" | "java" | "kt" | "c"
        | "h" | "cpp" | "cjs" | "mjs" => "text-x-script-symbolic",
        "mmd" => "network-workgroup-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

pub struct FileTree {
    pub list: gtk::ListBox,
    root: RefCell<Option<PathBuf>>,
    expanded: RefCell<HashSet<PathBuf>>,
    rows: RefCell<Vec<(gtk::ListBoxRow, PathBuf, bool)>>,
    selected: RefCell<Option<PathBuf>>,
    show_hidden: Cell<bool>,
    pub on_open: Callback<dyn Fn(&Path)>,
    me: RefCell<Weak<FileTree>>,
    /// Watches the shown folders so files created or removed (e.g. by Claude) show up.
    monitors: RefCell<Vec<(PathBuf, gio::FileMonitor)>>,
    pending: Cell<bool>,
}

fn entries(dir: &Path, hidden: bool) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    !HIDDEN.contains(&n.as_str()) && (hidden || !n.starts_with('.'))
                })
                .map(|e| {
                    let p = e.path();
                    let d = p.is_dir();
                    (p, d)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            let an = a.0.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let bn = b.0.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            an.cmp(&bn)
        })
    });
    v.truncate(MAX_ENTRIES);
    v
}

impl FileTree {
    pub fn new() -> Rc<FileTree> {
        let list = gtk::ListBox::new();
        list.add_css_class("wb-tree");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let t = Rc::new(FileTree {
            list,
            root: RefCell::new(None),
            expanded: RefCell::new(HashSet::new()),
            rows: RefCell::new(vec![]),
            selected: RefCell::new(None),
            show_hidden: Cell::new(crate::prefs::get("wb.files_hidden").as_deref() == Some("1")),
            on_open: RefCell::new(None),
            me: RefCell::new(Weak::new()),
            monitors: RefCell::new(vec![]),
            pending: Cell::new(false),
        });
        *t.me.borrow_mut() = Rc::downgrade(&t);
        let me = Rc::downgrade(&t);
        t.list.connect_row_activated(move |_, row| {
            let Some(t) = me.upgrade() else { return };
            let hit = t.rows.borrow().iter().find(|(r, _, _)| r == row).map(|(_, p, d)| (p.clone(), *d));
            let Some((path, is_dir)) = hit else { return };
            *t.selected.borrow_mut() = Some(path.clone());
            if is_dir {
                let mut e = t.expanded.borrow_mut();
                if !e.remove(&path) {
                    e.insert(path);
                }
                drop(e);
                t.rebuild();
            } else if let Some(f) = t.on_open.borrow().clone() {
                f(&path);
            }
        });
        t
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden.get()
    }

    pub fn set_show_hidden(&self, on: bool) {
        self.show_hidden.set(on);
        crate::prefs::set("wb.files_hidden", if on { "1" } else { "0" });
        self.rebuild();
    }

    pub fn root(&self) -> Option<PathBuf> {
        self.root.borrow().clone()
    }

    /// Follow a folder; a different folder starts collapsed.
    pub fn set_root(&self, dir: &Path) {
        if self.root.borrow().as_deref() == Some(dir) {
            return;
        }
        *self.root.borrow_mut() = Some(dir.to_path_buf());
        self.expanded.borrow_mut().clear();
        self.rebuild();
    }

    pub fn rebuild(&self) {
        // Removing every row resets the scroll position; put it back afterwards.
        let adj = self
            .list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            .map(|s| s.vadjustment());
        let scroll = adj.as_ref().map(|a| a.value());
        for (r, _, _) in self.rows.borrow_mut().drain(..) {
            self.list.remove(&r);
        }
        let Some(root) = self.root() else {
            self.monitors.borrow_mut().clear();
            return;
        };
        self.watch(&root);
        if let (Some(adj), Some(v)) = (adj, scroll) {
            glib::idle_add_local_once(move || adj.set_value(v.min(adj.upper() - adj.page_size()).max(0.0)));
        }
        let mut rows = vec![];
        self.add_dir(&root, 0, &mut rows);
        if rows.is_empty() {
            let r = gtk::ListBoxRow::new();
            r.set_activatable(false);
            r.set_selectable(false);
            let l = label("Empty folder", &["meta"]);
            l.set_margin_start(26);
            r.set_child(Some(&l));
            self.list.append(&r);
            rows.push((r, root.clone(), true));
        }
        let sel = self.selected.borrow().clone();
        for (r, p, _) in &rows {
            if Some(p) == sel.as_ref() {
                self.list.select_row(Some(r));
            }
        }
        *self.rows.borrow_mut() = rows;
    }

    /// Monitor the root and every expanded folder; a change rebuilds the tree once.
    fn watch(&self, root: &Path) {
        let mut want: Vec<PathBuf> = vec![root.to_path_buf()];
        want.extend(self.expanded.borrow().iter().filter(|p| p.starts_with(root)).cloned());
        let mut mons = self.monitors.borrow_mut();
        mons.retain(|(p, m)| {
            let keep = want.contains(p);
            if !keep {
                m.cancel();
            }
            keep
        });
        for dir in want {
            if mons.iter().any(|(p, _)| *p == dir) {
                continue;
            }
            let Ok(m) = gio::File::for_path(&dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { continue };
            let me = self.me.borrow().clone();
            m.connect_changed(move |_, _, _, ev| {
                if matches!(ev, gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::AttributeChanged) {
                    return;
                }
                let Some(t) = me.upgrade() else { return };
                if t.pending.replace(true) {
                    return;
                }
                let me = Rc::downgrade(&t);
                glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    if let Some(t) = me.upgrade() {
                        t.pending.set(false);
                        t.rebuild();
                    }
                });
            });
            mons.push((dir, m));
        }
    }

    fn add_dir(&self, dir: &Path, depth: i32, rows: &mut Vec<(gtk::ListBoxRow, PathBuf, bool)>) {
        let expanded = self.expanded.borrow().clone();
        for (p, is_dir) in entries(dir, self.show_hidden.get()) {
            let row = gtk::ListBoxRow::new();
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            b.set_margin_start(8 + depth * 12);
            let open = is_dir && expanded.contains(&p);
            let chev = gtk::Image::from_icon_name(if !is_dir {
                ""
            } else if open {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
            chev.set_pixel_size(12);
            chev.add_css_class("dim");
            chev.set_size_request(12, -1);
            b.append(&chev);
            let icon = gtk::Image::from_icon_name(icon_for(&p));
            icon.add_css_class("dim");
            b.append(&icon);
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let l = label(&name, if name.starts_with('.') { &["dim"] } else { &[] });
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_hexpand(true);
            b.append(&l);
            row.set_child(Some(&b));
            row.set_tooltip_text(Some(&p.to_string_lossy()));
            self.list.append(&row);
            rows.push((row, p.clone(), is_dir));
            if open {
                self.add_dir(&p, depth + 1, rows);
            }
        }
    }
}
