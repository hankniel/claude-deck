use crate::keys;
use crate::session::Tab;
use claude_deck::accounts::{self, Account};
use claude_deck::config::Config;
use claude_deck::pty::{self, Start};
use claude_deck::usage::{self, Usage, Window};
use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::{execute, terminal};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SIDEBAR_W: u16 = 30;
const PREFIX_HINT: &str = "C-a";

enum Mode {
    Normal,
    Prefix,
    Picker { purpose: Purpose, state: ListState },
    Help,
    ConfirmQuit,
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    NewTab,
    SwitchTab,
}

pub struct Options {
    pub account: Option<String>,
    pub claude_args: Vec<String>,
}

struct App {
    cfg: Config,
    accounts: Vec<Account>,
    emails: HashMap<String, Option<String>>,
    usage: HashMap<String, Usage>,
    tabs: Vec<Tab>,
    active: usize,
    mode: Mode,
    sidebar: bool,
    status: Option<(String, Instant)>,
    dirty: Arc<AtomicBool>,
    usage_rx: Receiver<(String, Usage)>,
    refresh_tx: Sender<()>,
    pane: Rect,
    launch_cwd: PathBuf,
    pending_args: Option<Vec<String>>,
    quit: bool,
    last_auto_check: Instant,
}

pub fn run(cfg: Config, opts: Options) -> Result<()> {
    let accounts = accounts::list();
    for a in &accounts {
        let _ = accounts::link_shared(a, &cfg);
    }
    let dirty = Arc::new(AtomicBool::new(true));
    let (usage_tx, usage_rx) = mpsc::channel();
    let (refresh_tx, refresh_rx) = mpsc::channel();
    spawn_poller(cfg.poll_secs, usage_tx, refresh_rx, dirty.clone());

    let mut terminal = ratatui::init();
    let enhanced = matches!(terminal::supports_keyboard_enhancement(), Ok(true));
    let mut out = std::io::stdout();
    let _ = execute!(out, EnableBracketedPaste);
    if enhanced {
        let _ = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let emails = accounts.iter().map(|a| (a.name.clone(), a.email())).collect();
    let mut app = App {
        cfg,
        accounts,
        emails,
        usage: HashMap::new(),
        tabs: vec![],
        active: 0,
        mode: Mode::Normal,
        sidebar: true,
        status: None,
        dirty,
        usage_rx,
        refresh_tx,
        pane: Rect::default(),
        launch_cwd: std::env::current_dir()?,
        pending_args: (!opts.claude_args.is_empty()).then_some(opts.claude_args),
        quit: false,
        last_auto_check: Instant::now(),
    };
    let size = terminal.size()?;
    app.layout(Rect::new(0, 0, size.width, size.height));
    app.open_first_tab(opts.account);

    let res = app.main_loop(&mut terminal);

    if enhanced {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    app.tabs.clear(); // kills children
    res
}

fn spawn_poller(
    poll_secs: u64,
    tx: Sender<(String, Usage)>,
    refresh: Receiver<()>,
    dirty: Arc<AtomicBool>,
) {
    std::thread::spawn(move || loop {
        for acc in accounts::list() {
            let u = usage::fetch(&acc);
            if tx.send((acc.name.clone(), u)).is_err() {
                return;
            }
            dirty.store(true, Ordering::Relaxed);
        }
        match refresh.recv_timeout(Duration::from_secs(poll_secs.max(30))) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {
                while refresh.try_recv().is_ok() {}
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    });
}

impl App {
    fn main_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            while let Ok((name, u)) = self.usage_rx.try_recv() {
                self.usage.insert(name, u);
                self.dirty.store(true, Ordering::Relaxed);
            }
            if self.last_auto_check.elapsed() >= Duration::from_secs(1) {
                self.last_auto_check = Instant::now();
                self.check_limits();
            }
            if let Some((_, t)) = &self.status {
                if t.elapsed() > Duration::from_secs(6) {
                    self.status = None;
                    self.dirty.store(true, Ordering::Relaxed);
                }
            }
            if self.dirty.swap(false, Ordering::Relaxed) {
                terminal.draw(|f| self.draw(f))?;
            }
            if event::poll(Duration::from_millis(16))? {
                // Drain everything queued so fast typing/pastes don't redraw per key.
                loop {
                    let ev = event::read()?;
                    self.handle_event(ev);
                    if self.quit || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
                self.dirty.store(true, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    // ---------- layout & tabs ----------

    fn layout(&mut self, area: Rect) {
        let main = if self.sidebar && area.width > SIDEBAR_W + 40 {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(SIDEBAR_W), Constraint::Min(10)])
                .split(area)[1]
        } else {
            area
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
            .split(main);
        self.pane = rows[1];
        for t in &mut self.tabs {
            t.resize(self.pane.height, self.pane.width);
        }
    }

    fn relayout(&mut self) {
        if let Ok((w, h)) = terminal::size() {
            self.layout(Rect::new(0, 0, w, h));
        }
    }

    fn reload_accounts(&mut self) {
        self.accounts = accounts::list();
        self.emails = self.accounts.iter().map(|a| (a.name.clone(), a.email())).collect();
    }

    fn account(&self, name: &str) -> Option<Account> {
        self.accounts.iter().find(|a| a.name == name).cloned()
    }

    fn current_cwd(&self) -> PathBuf {
        self.tabs.get(self.active).map(|t| t.cwd.clone()).unwrap_or_else(|| self.launch_cwd.clone())
    }

    fn open_first_tab(&mut self, account: Option<String>) {
        let cwd = self.launch_cwd.clone();
        let chosen = account
            .or_else(|| self.cfg.account_for(&cwd).filter(|a| self.account(a).is_some()))
            .or_else(|| (self.accounts.len() == 1).then(|| accounts::DEFAULT.to_string()));
        match chosen.and_then(|n| self.account(&n)) {
            Some(acc) => self.new_tab(&acc, cwd),
            None => self.open_picker(Purpose::NewTab),
        }
    }

    fn new_tab(&mut self, acc: &Account, cwd: PathBuf) {
        let start = match self.pending_args.take() {
            Some(a) => Start::Raw(a),
            None => Start::New,
        };
        let size = (self.pane.height, self.pane.width);
        match Tab::spawn(acc, &self.cfg, &cwd, size, start, self.dirty.clone()) {
            Ok(t) => {
                self.tabs.push(t);
                self.active = self.tabs.len() - 1;
            }
            Err(e) => self.flash(format!("failed to start claude: {e:#}")),
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.quit = true;
            return;
        }
        self.active = self.active.min(self.tabs.len() - 1);
    }

    fn claimed_sessions(&self, except: usize) -> Vec<String> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != except)
            .filter_map(|(_, t)| t.session_id.clone())
            .collect()
    }

    /// Restart tab `i` under `acc`, resuming its conversation.
    fn switch_tab(&mut self, i: usize, acc: &Account, reason: &str) {
        let claimed = self.claimed_sessions(i);
        let (cwd, session) = {
            let t = &self.tabs[i];
            (t.cwd.clone(), t.current_session(&claimed))
        };
        let from = self.tabs[i].account.clone();
        self.tabs[i].kill();
        let start = match session {
            Some(id) => Start::Resume(id),
            None => Start::Continue,
        };
        let size = (self.pane.height, self.pane.width);
        match Tab::spawn(acc, &self.cfg, &cwd, size, start, self.dirty.clone()) {
            Ok(mut t) => {
                t.last_switch = Some(Instant::now());
                self.tabs[i] = t;
                self.flash(format!("{reason}: tab {} {from} → {}", i + 1, acc.name));
            }
            Err(e) => self.flash(format!("switch failed: {e:#}")),
        }
    }

    fn best_account(&self, exclude: Option<&str>) -> Option<Account> {
        pty::best_account(&self.accounts, &self.usage, &self.cfg, exclude)
    }

    fn check_limits(&mut self) {
        let idle = Duration::from_secs(self.cfg.idle_secs);
        let cooldown = Duration::from_secs(self.cfg.cooldown_secs);
        for i in 0..self.tabs.len() {
            let t = &self.tabs[i];
            if t.is_exited() {
                continue;
            }
            let over = self.usage.get(&t.account).map(|u| u.over(&self.cfg)).unwrap_or(false);
            if !over {
                self.tabs[i].warned = false;
                continue;
            }
            if !self.cfg.auto_switch {
                if !t.warned {
                    let name = t.account.clone();
                    self.tabs[i].warned = true;
                    self.flash(format!("{name} is near its limit · {PREFIX_HINT} s to switch account"));
                }
                continue;
            }
            if t.idle_for() < idle || t.last_switch.is_some_and(|s| s.elapsed() < cooldown) {
                continue;
            }
            match self.best_account(Some(&t.account)) {
                Some(acc) => self.switch_tab(i, &acc, "auto-switch"),
                None if !t.warned => {
                    self.tabs[i].warned = true;
                    self.flash("all accounts are near their limits".into());
                }
                None => {}
            }
        }
    }

    fn flash(&mut self, msg: String) {
        self.status = Some((msg, Instant::now()));
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn open_picker(&mut self, purpose: Purpose) {
        self.reload_accounts();
        let cwd = self.current_cwd();
        let exclude = (purpose == Purpose::SwitchTab)
            .then(|| self.tabs.get(self.active).map(|t| t.account.clone()))
            .flatten();
        let preferred = self
            .cfg
            .account_for(&cwd)
            .filter(|n| Some(n) != exclude.as_ref())
            .or_else(|| self.best_account(exclude.as_deref()).map(|a| a.name));
        let idx = preferred
            .and_then(|n| self.accounts.iter().position(|a| a.name == n))
            .unwrap_or(0);
        let mut state = ListState::default();
        state.select(Some(idx));
        self.mode = Mode::Picker { purpose, state };
    }

    // ---------- input ----------

    fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Resize(w, h) => self.layout(Rect::new(0, 0, w, h)),
            Event::Paste(s) => {
                if matches!(self.mode, Mode::Normal) {
                    if let Some(t) = self.tabs.get_mut(self.active) {
                        t.paste(&s);
                    }
                }
            }
            Event::Key(k) if k.kind != KeyEventKind::Release => self.handle_key(k),
            _ => {}
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
        let is_prefix = k.code == KeyCode::Char('a') && k.modifiers == KeyModifiers::CONTROL;
        match &mut self.mode {
            Mode::Normal => {
                if is_prefix {
                    self.mode = Mode::Prefix;
                    return;
                }
                let Some(t) = self.tabs.get_mut(self.active) else { return };
                if k.modifiers.contains(KeyModifiers::SHIFT) && matches!(k.code, KeyCode::PageUp | KeyCode::PageDown) {
                    let page = (self.pane.height as usize / 2).max(1);
                    t.scroll = if k.code == KeyCode::PageUp { t.scroll + page } else { t.scroll.saturating_sub(page) };
                    return;
                }
                let app_cursor = t.parser.lock().unwrap().screen().application_cursor();
                t.write(&keys::encode(&k, app_cursor));
            }
            Mode::Prefix => {
                self.mode = Mode::Normal;
                self.prefix_command(k);
            }
            Mode::Help => self.mode = Mode::Normal,
            Mode::ConfirmQuit => {
                self.mode = Mode::Normal;
                if matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) {
                    self.quit = true;
                }
            }
            Mode::Picker { purpose, state } => {
                let purpose = *purpose;
                let n = self.accounts.len();
                let sel = state.selected().unwrap_or(0);
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => state.select(Some((sel + n - 1) % n)),
                    KeyCode::Down | KeyCode::Char('j') => state.select(Some((sel + 1) % n)),
                    KeyCode::Char(c @ '1'..='9') => {
                        let i = c as usize - '1' as usize;
                        if i < n {
                            self.pick(purpose, i);
                        }
                    }
                    KeyCode::Enter => self.pick(purpose, sel),
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.mode = Mode::Normal;
                        if self.tabs.is_empty() {
                            self.quit = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn pick(&mut self, purpose: Purpose, i: usize) {
        self.mode = Mode::Normal;
        let acc = self.accounts[i].clone();
        match purpose {
            Purpose::NewTab => {
                let cwd = self.current_cwd();
                self.new_tab(&acc, cwd);
            }
            Purpose::SwitchTab => {
                if self.tabs.get(self.active).is_some_and(|t| t.account != acc.name) {
                    self.switch_tab(self.active, &acc, "switched");
                }
            }
        }
    }

    fn prefix_command(&mut self, k: KeyEvent) {
        let n = self.tabs.len();
        match k.code {
            KeyCode::Char('a') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.write(&[0x01]);
                }
            }
            KeyCode::Char('c') => {
                let cwd = self.current_cwd();
                match self.cfg.account_for(&cwd).and_then(|a| self.account(&a)) {
                    Some(acc) => self.new_tab(&acc, cwd),
                    None => self.open_picker(Purpose::NewTab),
                }
            }
            KeyCode::Char('C') => self.open_picker(Purpose::NewTab),
            KeyCode::Char('s') if n > 0 => self.open_picker(Purpose::SwitchTab),
            KeyCode::Char('x') => self.close_tab(self.active),
            KeyCode::Char('R') if n > 0 => {
                if let Some(acc) = self.account(&self.tabs[self.active].account) {
                    self.switch_tab(self.active, &acc, "restarted");
                }
            }
            KeyCode::Char('n') | KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab if n > 0 => {
                self.active = (self.active + 1) % n
            }
            KeyCode::Char('p') | KeyCode::Char('h') | KeyCode::Left | KeyCode::BackTab if n > 0 => {
                self.active = (self.active + n - 1) % n
            }
            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i < n {
                    self.active = i;
                }
            }
            KeyCode::Char('a') => {
                self.cfg.auto_switch = !self.cfg.auto_switch;
                let _ = self.save_auto();
                let s = if self.cfg.auto_switch { "on" } else { "off" };
                self.flash(format!("auto-switch {s}"));
            }
            KeyCode::Char('b') => {
                self.sidebar = !self.sidebar;
                self.relayout();
            }
            KeyCode::Char('r') => {
                self.reload_accounts();
                let _ = self.refresh_tx.send(());
                self.flash("refreshing usage…".into());
            }
            KeyCode::Char('q') => self.mode = Mode::ConfirmQuit,
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }

    /// Persist only the auto-switch toggle, keeping other edits made on disk.
    fn save_auto(&self) -> Result<()> {
        let mut disk = Config::load()?;
        disk.auto_switch = self.cfg.auto_switch;
        disk.save()
    }

    // ---------- drawing ----------

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let show_sidebar = self.sidebar && area.width > SIDEBAR_W + 40;
        let (side, main) = if show_sidebar {
            let c = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(SIDEBAR_W), Constraint::Min(10)])
                .split(area);
            (Some(c[0]), c[1])
        } else {
            (None, area)
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
            .split(main);

        if let Some(s) = side {
            self.draw_sidebar(f, s);
        }
        self.draw_tabbar(f, rows[0]);
        self.draw_pane(f, rows[1]);
        self.draw_status(f, rows[2]);

        match &mut self.mode {
            Mode::Picker { purpose, state } => {
                let purpose = *purpose;
                let mut st = state.clone();
                self.draw_picker(f, purpose, &mut st);
                if let Mode::Picker { state, .. } = &mut self.mode {
                    *state = st;
                }
            }
            Mode::Help => draw_help(f),
            Mode::ConfirmQuit => {
                let r = centered(f.area(), 44, 3);
                f.render_widget(Clear, r);
                f.render_widget(
                    Paragraph::new(" Quit and end all sessions? (y/N)")
                        .block(Block::default().borders(Borders::ALL)),
                    r,
                );
            }
            _ => {}
        }
    }

    fn draw_tabbar(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![];
        for (i, t) in self.tabs.iter().enumerate() {
            let dir = t.cwd.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mark = if t.is_exited() { " ✗" } else { "" };
            let label = format!(" {} {} · {}{} ", i + 1, t.account, dir, mark);
            let style = if i == self.active {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
        }
        if matches!(self.mode, Mode::Prefix) {
            spans.push(Span::styled(" PREFIX ", Style::default().fg(Color::Black).bg(Color::Yellow)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let text = match &self.status {
            Some((m, _)) => Span::styled(format!(" {m}"), Style::default().fg(Color::Yellow)),
            None => {
                let ended = self.tabs.get(self.active).is_some_and(|t| t.is_exited());
                let hint = if ended {
                    format!(" session ended · {PREFIX_HINT} R restart · {PREFIX_HINT} x close")
                } else {
                    format!(" {PREFIX_HINT} ? help · c new · s switch account · a auto · q quit")
                };
                Span::styled(hint, Style::default().fg(Color::DarkGray))
            }
        };
        f.render_widget(Paragraph::new(Line::from(text)), area);
    }

    fn draw_pane(&mut self, f: &mut Frame, area: Rect) {
        let Some(t) = self.tabs.get_mut(self.active) else {
            f.render_widget(
                Paragraph::new("  no sessions · C-a c to start one").style(Style::default().fg(Color::DarkGray)),
                area,
            );
            return;
        };
        let mut p = t.parser.lock().unwrap();
        p.set_scrollback(t.scroll);
        t.scroll = p.screen().scrollback();
        let screen = p.screen();
        let buf = f.buffer_mut();
        let (rows, cols) = screen.size();
        for r in 0..rows.min(area.height) {
            for c in 0..cols.min(area.width) {
                let Some(cell) = screen.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let Some(out) = buf.cell_mut((area.x + c, area.y + r)) else { continue };
                let s = cell.contents();
                out.set_symbol(if s.is_empty() { " " } else { &s });
                let mut style = Style::default().fg(color(cell.fgcolor())).bg(color(cell.bgcolor()));
                let mut m = Modifier::empty();
                if cell.bold() {
                    m |= Modifier::BOLD;
                }
                if cell.italic() {
                    m |= Modifier::ITALIC;
                }
                if cell.underline() {
                    m |= Modifier::UNDERLINED;
                }
                if cell.inverse() {
                    m |= Modifier::REVERSED;
                }
                style = style.add_modifier(m);
                out.set_style(style);
            }
        }
        let cursor_visible = t.scroll == 0 && !screen.hide_cursor() && matches!(self.mode, Mode::Normal);
        if cursor_visible {
            let (cr, cc) = screen.cursor_position();
            if cr < area.height && cc < area.width {
                f.set_cursor_position((area.x + cc, area.y + cr));
            }
        }
        if t.scroll > 0 {
            let tag = format!(" scrollback +{} ", t.scroll);
            let w = tag.len() as u16;
            if area.width > w {
                let r = Rect::new(area.x + area.width - w, area.y, w, 1);
                f.render_widget(Paragraph::new(tag).style(Style::default().fg(Color::Black).bg(Color::Yellow)), r);
            }
        }
    }

    fn account_lines(&self, acc: &Account, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![];
        let u = self.usage.get(&acc.name).cloned().unwrap_or(Usage::Unknown);
        let plan = match &u {
            Usage::Ok { plan: Some(p), .. } => format!(" {p}"),
            _ => String::new(),
        };
        let disabled = if self.cfg.disabled.contains(&acc.name) { " (off)" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(acc.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{plan}{disabled}"), Style::default().fg(Color::DarkGray)),
        ]));
        if let Some(Some(e)) = self.emails.get(&acc.name) {
            lines.push(Line::styled(truncate(e, width), Style::default().fg(Color::DarkGray)));
        }
        match &u {
            Usage::Ok { five, week, .. } => {
                for (label, w, th) in [("5h", five, self.cfg.threshold), ("7d", week, self.cfg.weekly_threshold)] {
                    if let Some(w) = w {
                        lines.push(gauge_line(label, w, th, width));
                    }
                }
            }
            Usage::Error(e) => lines.push(Line::styled(truncate(&format!("error: {e}"), width), Style::default().fg(Color::Red))),
            other => lines.push(Line::styled(other.label(), Style::default().fg(Color::DarkGray))),
        }
        lines
    }

    fn draw_sidebar(&self, f: &mut Frame, area: Rect) {
        let active_acc = self.tabs.get(self.active).map(|t| t.account.as_str());
        let inner_w = area.width.saturating_sub(4) as usize;
        let mut lines: Vec<Line> = vec![];
        for acc in &self.accounts {
            let mut al = self.account_lines(acc, inner_w);
            let marker = if Some(acc.name.as_str()) == active_acc { "● " } else { "  " };
            if let Some(first) = al.first_mut() {
                first.spans.insert(0, Span::styled(marker, Style::default().fg(Color::Cyan)));
            }
            for l in al.iter_mut().skip(1) {
                l.spans.insert(0, Span::raw("  "));
            }
            lines.extend(al);
            lines.push(Line::raw(""));
        }
        let auto = if self.cfg.auto_switch {
            Span::styled("auto-switch ON", Style::default().fg(Color::Green))
        } else {
            Span::styled("auto-switch off", Style::default().fg(Color::DarkGray))
        };
        lines.push(Line::from(auto));
        lines.push(Line::styled(
            format!("at 5h {:.0}% · 7d {:.0}%", self.cfg.threshold, self.cfg.weekly_threshold),
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" accounts ")),
            area,
        );
    }

    fn draw_picker(&self, f: &mut Frame, purpose: Purpose, state: &mut ListState) {
        let width = 56u16.min(f.area().width.saturating_sub(4));
        let items: Vec<ListItem> = self
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut ls = self.account_lines(a, width.saturating_sub(8) as usize);
                if let Some(first) = ls.first_mut() {
                    first.spans.insert(0, Span::styled(format!("{} ", i + 1), Style::default().fg(Color::Cyan)));
                }
                for l in ls.iter_mut().skip(1) {
                    l.spans.insert(0, Span::raw("  "));
                }
                ls.push(Line::raw(""));
                ListItem::new(ls)
            })
            .collect();
        let height: u16 = items.iter().map(|i| i.height() as u16).sum::<u16>() + 4;
        let r = centered(f.area(), width, height.min(f.area().height.saturating_sub(2)));
        let cwd = self.current_cwd();
        let title = match purpose {
            Purpose::NewTab => format!(" new session in {} ", cwd.file_name().unwrap_or_default().to_string_lossy()),
            Purpose::SwitchTab => " move this session to… ".to_string(),
        };
        f.render_widget(Clear, r);
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_bottom(" ↑↓/1-9 choose · enter · esc · add: claude-deck add <name> "),
            )
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, r, state);
    }
}

fn draw_help(f: &mut Frame) {
    let text = [
        "Prefix: Ctrl-a, then",
        "",
        "  c       new session (mapped account or picker)",
        "  C       new session, always pick the account",
        "  s       move this session to another account",
        "  R       restart this session (resumes the chat)",
        "  x       close this session",
        "  n / p   next / previous tab   (also ← →)",
        "  1-9     go to tab",
        "  a       toggle auto-switch",
        "  r       refresh usage",
        "  b       toggle sidebar",
        "  C-a     send a literal Ctrl-a",
        "  q       quit",
        "",
        "Shift-PgUp / Shift-PgDn  scroll back",
        "",
        "any key to close",
    ];
    let r = centered(f.area(), 54, text.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(text.iter().map(|s| Line::raw(*s)).collect::<Vec<_>>())
            .block(Block::default().borders(Borders::ALL).title(" claude-deck ")),
        r,
    );
}

fn gauge_line(label: &str, w: &Window, threshold: f64, width: usize) -> Line<'static> {
    let pct = w.current_pct();
    let reset = w.resets_at.map(usage::until).unwrap_or_default();
    let bar_w = width.saturating_sub(label.len() + 6 + reset.len() + 2).clamp(4, 20);
    let filled = ((pct / 100.0) * bar_w as f64).round().clamp(0.0, bar_w as f64) as usize;
    let color = if pct >= threshold {
        Color::Red
    } else if pct >= threshold * 0.75 {
        Color::Yellow
    } else {
        Color::Green
    };
    Line::from(vec![
        Span::raw(format!("{label} ")),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(bar_w - filled), Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {pct:>3.0}%"), Style::default().fg(color)),
        Span::styled(format!(" {reset}"), Style::default().fg(Color::DarkGray)),
    ])
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
