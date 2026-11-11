//! Mermaid `sequenceDiagram` subset: participants/actors (with aliases), messages with every
//! arrow kind, notes, loop/alt/else/opt/par/and/critical/option/break/rect blocks, autonumber.

#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub id: String,
    pub label: String,
    pub actor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Head {
    Arrow,
    Open,
    Cross,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Side {
    Left,
    Right,
    Over,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Message { from: usize, to: usize, text: String, dashed: bool, head: Head, number: Option<usize> },
    Note { side: Side, over: Vec<usize>, text: String },
    Start { kind: String, label: String },
    Else { label: String },
    End,
}

#[derive(Clone, Debug, Default)]
pub struct Sequence {
    pub title: String,
    pub participants: Vec<Participant>,
    pub events: Vec<Event>,
    pub errors: Vec<String>,
}

impl Sequence {
    fn participant(&mut self, id: &str) -> usize {
        if let Some(i) = self.participants.iter().position(|p| p.id == id) {
            return i;
        }
        self.participants.push(Participant { id: id.into(), label: id.into(), actor: false });
        self.participants.len() - 1
    }
}

const ARROWS: &[(&str, bool, Head)] = &[
    ("-->>", true, Head::Arrow),
    ("->>", false, Head::Arrow),
    ("--x", true, Head::Cross),
    ("-x", false, Head::Cross),
    ("--)", true, Head::Open),
    ("-)", false, Head::Open),
    ("-->", true, Head::None),
    ("->", false, Head::None),
];

fn clean(s: &str) -> String {
    s.trim().replace("<br/>", "\n").replace("<br>", "\n").replace("#59;", ";")
}

pub fn parse(text: &str) -> Sequence {
    let mut sq = Sequence::default();
    let mut header = false;
    let mut number: Option<usize> = None;
    for (no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if !header {
            if line.starts_with("sequenceDiagram") {
                header = true;
            }
            continue;
        }
        let lower = line.to_lowercase();
        let word = lower.split_whitespace().next().unwrap_or("");
        let rest_of = |w: &str| line[w.len()..].trim().to_string();
        match word {
            "participant" | "actor" => {
                let r = rest_of(word);
                let (id, label) = match r.split_once(" as ") {
                    Some((i, l)) => (i.trim().to_string(), clean(l)),
                    None => (r.clone(), r.clone()),
                };
                let i = sq.participant(&id);
                sq.participants[i].label = label;
                sq.participants[i].actor = word == "actor";
                continue;
            }
            "autonumber" => {
                number = Some(1);
                continue;
            }
            "activate" | "deactivate" | "box" | "create" | "destroy" | "links" | "link" => continue,
            "title" | "title:" => {
                sq.title = line[5..].trim_start_matches(':').trim().to_string();
                continue;
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                sq.events.push(Event::Start { kind: word.into(), label: rest_of(word) });
                continue;
            }
            "else" | "and" | "option" => {
                sq.events.push(Event::Else { label: rest_of(word) });
                continue;
            }
            "end" => {
                sq.events.push(Event::End);
                continue;
            }
            "note" => {
                let r = rest_of("note");
                let Some((place, text)) = r.split_once(':') else {
                    sq.errors.push(format!("line {}: note needs `: text`", no + 1));
                    continue;
                };
                let p = place.trim();
                let pl = p.to_lowercase();
                let (side, who) = if let Some(w) = pl.strip_prefix("right of") {
                    (Side::Right, p[p.len() - w.len()..].trim())
                } else if let Some(w) = pl.strip_prefix("left of") {
                    (Side::Left, p[p.len() - w.len()..].trim())
                } else if let Some(w) = pl.strip_prefix("over") {
                    (Side::Over, p[p.len() - w.len()..].trim())
                } else {
                    sq.errors.push(format!("line {}: note must be left of, right of or over", no + 1));
                    continue;
                };
                let over = who.split(',').map(|w| sq.participant(w.trim())).collect();
                sq.events.push(Event::Note { side, over, text: clean(text) });
                continue;
            }
            _ => {}
        }
        // message: A->>B: text
        let Some((head, text)) = line.split_once(':') else {
            sq.errors.push(format!("line {}: can't read {line}", no + 1));
            continue;
        };
        let mut found = None;
        for (a, dashed, h) in ARROWS {
            if let Some(i) = head.find(a) {
                found = Some((i, a.len(), *dashed, *h));
                break;
            }
        }
        let Some((i, len, dashed, h)) = found else {
            sq.errors.push(format!("line {}: no arrow in {line}", no + 1));
            continue;
        };
        let from = head[..i].trim();
        let to = head[i + len..].trim().trim_start_matches(['+', '-']).trim();
        if from.is_empty() || to.is_empty() {
            sq.errors.push(format!("line {}: message needs two participants", no + 1));
            continue;
        }
        let (f, t) = (sq.participant(from), sq.participant(to));
        sq.events.push(Event::Message { from: f, to: t, text: clean(text), dashed, head: h, number });
        if let Some(n) = number.as_mut() {
            *n += 1;
        }
    }
    if !header {
        sq.errors.push("not a sequenceDiagram".into());
    }
    sq
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example() {
        let s = parse(
            "sequenceDiagram\n  autonumber\n  actor U as User\n  participant API\n  U->>+API: POST /orders\n  alt valid\n    API-->>U: 201\n  else invalid\n    API--xU: 400\n  end\n  Note over U,API: done\n  API-)Queue: event\n",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.participants.len(), 3);
        assert!(s.participants[0].actor);
        assert_eq!(s.participants[0].label, "User");
        assert_eq!(s.events.len(), 8);
        assert!(matches!(&s.events[0], Event::Message { from: 0, to: 1, number: Some(1), head: Head::Arrow, dashed: false, .. }));
        assert!(matches!(&s.events[4], Event::Message { head: Head::Cross, dashed: true, .. }));
        assert!(matches!(&s.events[6], Event::Note { side: Side::Over, .. }));
        assert!(matches!(&s.events[7], Event::Message { head: Head::Open, to: 2, .. }));
    }
}
