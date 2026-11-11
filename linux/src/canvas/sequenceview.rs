//! Mermaid sequence diagrams: participants with lifelines, messages, notes and blocks.

use super::draw::{self, set, BODY, BOLD, SMALL, SMALL_BOLD};
use super::view::Model;
use claude_deck::canvas::sequence::{Event, Head, Sequence, Side};
use gtk::cairo;

const HEAD_H: f64 = 40.0;
const TOP: f64 = 12.0;

struct Row {
    y: f64,
    h: f64,
}

pub struct SeqModel {
    seq: Sequence,
    xs: Vec<f64>,
    widths: Vec<f64>,
    rows: Vec<Row>,
    width: f64,
    height: f64,
    hover: Option<usize>,
}

impl SeqModel {
    pub fn new(seq: Sequence) -> SeqModel {
        let meas = gtk::Label::new(None);
        let widths: Vec<f64> = seq.participants.iter().map(|p| (draw::measure(&meas, &p.label, BOLD).0 + 32.0).max(110.0)).collect();
        // gap between neighbours: wide enough for the longest message between them
        let n = widths.len();
        let mut gaps = vec![70.0f64; n.saturating_sub(1)];
        for e in &seq.events {
            if let Event::Message { from, to, text, .. } = e {
                let (a, b) = ((*from).min(*to), (*from).max(*to));
                if a != b {
                    let need = draw::measure(&meas, text, SMALL).0 + 40.0;
                    let span = (b - a) as f64;
                    for g in gaps.iter_mut().take(b).skip(a) {
                        *g = g.max(need / span - 40.0);
                    }
                } else if a + 1 < n {
                    gaps[a] = gaps[a].max(draw::measure(&meas, text, SMALL).0 + 30.0);
                }
            }
        }
        let mut xs = vec![];
        let mut x = 20.0;
        for i in 0..n {
            x += widths[i] / 2.0;
            xs.push(x);
            x += widths[i] / 2.0 + gaps.get(i).copied().unwrap_or(0.0);
        }
        let width = x + 20.0;
        let mut rows = vec![];
        let mut y = TOP + HEAD_H + 24.0;
        for e in &seq.events {
            let h = match e {
                Event::Message { from, to, text, .. } => {
                    let th = draw::measure(&meas, text, SMALL).1;
                    if from == to {
                        th + 44.0
                    } else {
                        th + 26.0
                    }
                }
                Event::Note { text, .. } => draw::measure(&meas, text, SMALL).1 + 26.0,
                Event::Start { .. } => 34.0,
                Event::Else { .. } => 28.0,
                Event::End => 14.0,
            };
            rows.push(Row { y, h });
            y += h;
        }
        let height = y + 16.0 + HEAD_H + TOP;
        SeqModel { seq, xs, widths, rows, width, height, hover: None }
    }

    fn head_box(&self, cr: &cairo::Context, w: &gtk::DrawingArea, i: usize, y: f64) {
        let t = draw::theme();
        let p = &self.seq.participants[i];
        let (x, bw) = (self.xs[i], self.widths[i]);
        set(cr, t.card, 1.0);
        draw::rounded(cr, x - bw / 2.0, y, bw, HEAD_H, 8.0);
        let _ = cr.fill_preserve();
        set(cr, if self.hover == Some(i) { t.accent } else { t.border }, 1.0);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let label = if p.actor { format!("👤 {}", p.label) } else { p.label.clone() };
        draw::text_centered(cr, w, &label, x, y + HEAD_H / 2.0, BOLD, t.fg, 1.0);
    }
}

impl Model for SeqModel {
    fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    fn draw(&self, cr: &cairo::Context, w: &gtk::DrawingArea) {
        let t = draw::theme();
        let bottom = self.height - TOP - HEAD_H;
        // lifelines
        for &x in &self.xs {
            set(cr, t.edge, 0.6);
            cr.set_line_width(1.0);
            cr.set_dash(&[4.0, 4.0], 0.0);
            cr.move_to(x, TOP + HEAD_H);
            cr.line_to(x, bottom);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        // blocks: frames from Start to End, with else separators
        let mut stack: Vec<(usize, String, String)> = vec![];
        let (left, right) = (8.0, self.width - 8.0);
        for (k, e) in self.seq.events.iter().enumerate() {
            match e {
                Event::Start { kind, label } => stack.push((k, kind.clone(), label.clone())),
                Event::End => {
                    if let Some((s, kind, label)) = stack.pop() {
                        let d = stack.len() as f64 * 8.0;
                        let (y0, y1) = (self.rows[s].y + 4.0, self.rows[k].y + self.rows[k].h / 2.0);
                        set(cr, t.muted, if kind == "rect" { 0.08 } else { 0.035 });
                        draw::rounded(cr, left + d, y0, right - left - 2.0 * d, y1 - y0, 6.0);
                        let _ = cr.fill_preserve();
                        set(cr, t.muted, 0.7);
                        cr.set_line_width(1.0);
                        let _ = cr.stroke();
                        if kind != "rect" {
                            let tag = format!(" {kind} ");
                            let (tw, th) = draw::measure(w, &tag, SMALL_BOLD);
                            set(cr, t.muted, 0.22);
                            cr.rectangle(left + d, y0, tw + 6.0, th + 6.0);
                            let _ = cr.fill();
                            draw::text(cr, w, &tag, left + d + 3.0, y0 + 3.0, SMALL_BOLD, t.fg, 1.0, 0.0, None);
                            if !label.is_empty() {
                                draw::text(cr, w, &format!("[{label}]"), left + d + tw + 14.0, y0 + 3.0, SMALL, t.muted, 1.0, 0.0, None);
                            }
                        }
                        // else lines between s and k at this depth
                        let mut depth = 0;
                        for j in s + 1..k {
                            match &self.seq.events[j] {
                                Event::Start { .. } => depth += 1,
                                Event::End => depth -= 1,
                                Event::Else { label } if depth == 0 => {
                                    let y = self.rows[j].y + 6.0;
                                    set(cr, t.muted, 0.7);
                                    cr.set_dash(&[5.0, 4.0], 0.0);
                                    cr.move_to(left + d, y);
                                    cr.line_to(right - d, y);
                                    let _ = cr.stroke();
                                    cr.set_dash(&[], 0.0);
                                    if !label.is_empty() {
                                        draw::text(cr, w, &format!("[{label}]"), left + d + 8.0, y + 3.0, SMALL, t.muted, 1.0, 0.0, None);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // messages and notes
        for (k, e) in self.seq.events.iter().enumerate() {
            let r = &self.rows[k];
            match e {
                Event::Message { from, to, text, dashed, head, number } => {
                    let (x1, x2) = (self.xs[*from], self.xs[*to]);
                    let label = match number {
                        Some(n) => format!("{n}. {text}"),
                        None => text.clone(),
                    };
                    let (_, th) = draw::measure(w, &label, SMALL);
                    let hot = self.hover.is_some_and(|h| h == *from || h == *to);
                    let c = if hot { t.accent } else { t.fg };
                    set(cr, c, 0.9);
                    cr.set_line_width(1.3);
                    if *dashed {
                        cr.set_dash(&[5.0, 4.0], 0.0);
                    }
                    let (tip, from_pt);
                    if from == to {
                        let y0 = r.y + th + 8.0;
                        cr.move_to(x1, y0);
                        cr.line_to(x1 + 36.0, y0);
                        cr.line_to(x1 + 36.0, y0 + 18.0);
                        cr.line_to(x1 + 2.0, y0 + 18.0);
                        tip = (x1 + 2.0, y0 + 18.0);
                        from_pt = (x1 + 36.0, y0 + 18.0);
                        draw::text(cr, w, &label, x1 + 8.0, r.y + 2.0, SMALL, t.fg, 1.0, 0.0, None);
                    } else {
                        let y = r.y + th + 10.0;
                        let dir = if x2 > x1 { 1.0 } else { -1.0 };
                        cr.move_to(x1, y);
                        cr.line_to(x2 - dir * 2.0, y);
                        tip = (x2 - dir * 2.0, y);
                        from_pt = (x1, y);
                        draw::text(cr, w, &label, (x1 + x2) / 2.0, r.y + 3.0, SMALL, t.fg, 1.0, 0.5, None);
                    }
                    let _ = cr.stroke();
                    cr.set_dash(&[], 0.0);
                    set(cr, c, 0.9);
                    match head {
                        Head::Arrow => draw::arrow_head(cr, tip, from_pt, 9.0),
                        Head::Open => {
                            let ang = (tip.1 - from_pt.1).atan2(tip.0 - from_pt.0);
                            for a in [ang + std::f64::consts::PI - 0.5, ang + std::f64::consts::PI + 0.5] {
                                cr.move_to(tip.0, tip.1);
                                cr.line_to(tip.0 + 9.0 * a.cos(), tip.1 + 9.0 * a.sin());
                            }
                            let _ = cr.stroke();
                        }
                        Head::Cross => {
                            cr.move_to(tip.0 - 5.0, tip.1 - 5.0);
                            cr.line_to(tip.0 + 5.0, tip.1 + 5.0);
                            cr.move_to(tip.0 - 5.0, tip.1 + 5.0);
                            cr.line_to(tip.0 + 5.0, tip.1 - 5.0);
                            let _ = cr.stroke();
                        }
                        Head::None => {}
                    }
                }
                Event::Note { side, over, text } => {
                    let (tw, th) = draw::measure(w, text, SMALL);
                    let xs: Vec<f64> = over.iter().map(|&i| self.xs[i]).collect();
                    let (lo, hi) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
                    let bw = tw + 20.0;
                    let x0 = match side {
                        Side::Left => lo - bw - 10.0,
                        Side::Right => lo + 10.0,
                        Side::Over => ((lo + hi) / 2.0 - bw / 2.0).min(lo - 20.0).max(lo - bw / 2.0 - (hi - lo) / 2.0 - 20.0),
                    };
                    let bw = if matches!(side, Side::Over) && over.len() > 1 { (hi - lo + 40.0).max(bw) } else { bw };
                    let x0 = if matches!(side, Side::Over) { (lo + hi) / 2.0 - bw / 2.0 } else { x0 };
                    let note = if t.dark { draw::mix(t.card, (0.95, 0.8, 0.35), 0.18) } else { (1.0, 0.97, 0.82) };
                    set(cr, note, 1.0);
                    draw::rounded(cr, x0, r.y + 4.0, bw, th + 14.0, 4.0);
                    let _ = cr.fill_preserve();
                    set(cr, t.border, 1.0);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                    draw::text(cr, w, text, x0 + 10.0, r.y + 11.0, SMALL, t.fg, 1.0, 0.0, None);
                }
                _ => {}
            }
        }
        for i in 0..self.seq.participants.len() {
            self.head_box(cr, w, i, TOP);
            self.head_box(cr, w, i, bottom);
        }
        if self.seq.participants.is_empty() {
            draw::text(cr, w, "Empty sequence diagram", 20.0, 20.0, BODY, t.muted, 1.0, 0.0, None);
        }
    }

    fn hover(&mut self, p: Option<(f64, f64)>) -> bool {
        let h = p.and_then(|(x, _)| self.xs.iter().enumerate().position(|(i, &cx)| (x - cx).abs() < self.widths[i] / 2.0));
        if h != self.hover {
            self.hover = h;
            return true;
        }
        false
    }
}
