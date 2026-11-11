//! `.dbml` — database diagrams in DBML (dbdiagram.io syntax): tables, columns with settings,
//! inline and standalone refs, enums, table groups, notes and header colours.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub name: String,
    pub ty: String,
    pub pk: bool,
    pub not_null: bool,
    pub unique: bool,
    pub increment: bool,
    pub default: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub name: String,
    pub alias: Option<String>,
    pub columns: Vec<Column>,
    pub note: Option<String>,
    pub color: Option<String>,
    pub group: Option<usize>,
}

impl Table {
    /// ("schema", "table"); schema is empty when not given.
    pub fn split_name(&self) -> (String, String) {
        match self.name.rsplit_once('.') {
            Some((s, t)) => (s.to_string(), t.to_string()),
            None => (String::new(), self.name.clone()),
        }
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    One,
    Many,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    pub from: (usize, usize),
    pub to: (usize, usize),
    pub from_card: Card,
    pub to_card: Card,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Enum {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub name: String,
    pub color: Option<String>,
    pub tables: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Erd {
    pub project: Option<String>,
    pub tables: Vec<Table>,
    pub refs: Vec<Ref>,
    pub enums: Vec<Enum>,
    pub groups: Vec<Group>,
    pub errors: Vec<String>,
}

impl Erd {
    pub fn table(&self, name: &str) -> Option<usize> {
        let name = unquote(name);
        self.tables
            .iter()
            .position(|t| t.name == name || t.alias.as_deref() == Some(name.as_str()))
            .or_else(|| self.tables.iter().position(|t| t.split_name().1 == name && !name.contains('.')))
            .or_else(|| name.strip_prefix("public.").and_then(|n| self.tables.iter().position(|t| t.name == n)))
    }

    pub fn is_fk(&self, t: usize, c: usize) -> bool {
        self.refs.iter().any(|r| (r.from == (t, c) && r.from_card == Card::Many) || (r.to == (t, c) && r.to_card == Card::Many))
            || self.refs.iter().any(|r| r.from == (t, c) || r.to == (t, c)) && !self.tables[t].columns[c].pk
    }

    pub fn is_enum(&self, ty: &str) -> bool {
        let ty = unquote(ty);
        self.enums.iter().any(|e| e.name == ty || e.name.rsplit('.').next() == Some(ty.as_str()))
    }
}

pub fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\'', '`'] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// Split on `sep` outside quotes, brackets and parentheses.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q: Option<char> = None;
    let mut depth = 0i32;
    for ch in s.chars() {
        if let Some(c) = q {
            cur.push(ch);
            if ch == c {
                q = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                q = Some(ch);
                cur.push(ch);
            }
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(ch);
            }
            c if c == sep && depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Remove comments and fold ''' multi-line strings into single-line '…' strings.
fn clean_lines(text: &str) -> Vec<(usize, String)> {
    #[derive(PartialEq)]
    enum St {
        Code,
        Quote(char),
        Triple,
        Block,
        LineComment,
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut cur = String::new();
    let mut line_no = 1;
    let mut start_line = 1;
    let mut st = St::Code;
    let mut i = 0;
    let flush = |cur: &mut String, out: &mut Vec<(usize, String)>, at: usize| {
        let t = cur.trim();
        if !t.is_empty() {
            out.push((at, t.to_string()));
        }
        cur.clear();
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let triple = c == '\'' && next == Some('\'') && chars.get(i + 2) == Some(&'\'');
        if c == '\n' {
            line_no += 1;
        }
        match st {
            St::Code => {
                if c == '\n' {
                    flush(&mut cur, &mut out, start_line);
                    start_line = line_no;
                } else if triple {
                    st = St::Triple;
                    cur.push('\'');
                    i += 3;
                    continue;
                } else if c == '/' && next == Some('/') {
                    st = St::LineComment;
                } else if c == '/' && next == Some('*') {
                    st = St::Block;
                    i += 2;
                    continue;
                } else {
                    if c == '\'' || c == '"' || c == '`' {
                        st = St::Quote(c);
                    }
                    cur.push(c);
                }
            }
            St::Quote(q) => {
                if c == '\n' {
                    // unterminated: end the string at the line break
                    st = St::Code;
                    flush(&mut cur, &mut out, start_line);
                    start_line = line_no;
                } else {
                    if c == q {
                        st = St::Code;
                    }
                    cur.push(c);
                }
            }
            St::Triple => {
                if triple {
                    st = St::Code;
                    cur.push('\'');
                    i += 3;
                    continue;
                }
                match c {
                    '\n' => {
                        if !cur.ends_with(' ') && !cur.ends_with('\'') {
                            cur.push(' ');
                        }
                    }
                    '\'' => cur.push('’'),
                    c if c.is_whitespace() && (cur.ends_with(' ') || cur.ends_with('\'')) => {}
                    c => cur.push(c),
                }
            }
            St::Block => {
                if c == '*' && next == Some('/') {
                    st = St::Code;
                    i += 2;
                    continue;
                }
            }
            St::LineComment => {
                if c == '\n' {
                    st = St::Code;
                    flush(&mut cur, &mut out, start_line);
                    start_line = line_no;
                }
            }
        }
        i += 1;
    }
    flush(&mut cur, &mut out, start_line);
    out
}

/// `[a, b: c]` at the end of `s` → (before, settings)
fn take_settings(s: &str) -> (String, Vec<String>) {
    let s = s.trim();
    if !s.ends_with(']') {
        return (s.to_string(), vec![]);
    }
    let mut depth = 0;
    let mut q: Option<char> = None;
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for &(i, c) in chars.iter().rev() {
        if let Some(qc) = q {
            if c == qc {
                q = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => q = Some(c),
            ']' => depth += 1,
            '[' => {
                depth -= 1;
                if depth == 0 {
                    return (s[..i].trim().to_string(), split_top(&s[i + 1..s.len() - 1], ','));
                }
            }
            _ => {}
        }
    }
    (s.to_string(), vec![])
}

fn setting_value<'a>(settings: &'a [String], key: &str) -> Option<&'a str> {
    settings.iter().find_map(|s| {
        let (k, v) = s.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim())
    })
}

/// First token of a line, honouring quotes and parentheses: (token, rest).
fn token(s: &str) -> (String, String) {
    let s = s.trim_start();
    let mut depth = 0;
    let mut q: Option<char> = None;
    for (i, c) in s.char_indices() {
        if let Some(qc) = q {
            if c == qc {
                q = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => q = Some(c),
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => return (s[..i].to_string(), s[i..].trim_start().to_string()),
            _ => {}
        }
    }
    (s.to_string(), String::new())
}

/// A table name possibly written `schema.table` or `"schema"."table"`.
fn table_name(s: &str) -> String {
    s.split('.').map(unquote).collect::<Vec<_>>().join(".")
}

struct PendingRef {
    line: usize,
    from: String,
    op: String,
    to: String,
    name: Option<String>,
}

fn ref_parts(spec: &str) -> Option<(String, String, String)> {
    for op in ["<>", ">", "<", "-"] {
        // the operator surrounded by spaces, or directly between names
        if let Some(i) = spec.find(&format!(" {op} ")) {
            return Some((spec[..i].trim().into(), op.into(), spec[i + op.len() + 2..].trim().into()));
        }
    }
    for op in ["<>", ">", "<", "-"] {
        if let Some(i) = spec.find(op) {
            return Some((spec[..i].trim().into(), op.into(), spec[i + op.len()..].trim().into()));
        }
    }
    None
}

fn cards(op: &str) -> (Card, Card) {
    match op {
        ">" => (Card::Many, Card::One),
        "<" => (Card::One, Card::Many),
        "<>" => (Card::Many, Card::Many),
        _ => (Card::One, Card::One),
    }
}

pub fn parse(text: &str) -> Erd {
    let mut erd = Erd::default();
    let lines = clean_lines(text);
    let mut refs: Vec<PendingRef> = vec![];
    let mut group_members: Vec<(usize, Vec<String>)> = vec![];
    let mut i = 0;
    let skip_block = |i: &mut usize, lines: &[(usize, String)]| {
        let mut depth = 1i32;
        while *i < lines.len() && depth > 0 {
            let l = &lines[*i].1;
            depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            *i += 1;
        }
    };
    while i < lines.len() {
        let (no, line) = lines[i].clone();
        i += 1;
        let (mut kw, mut rest) = token(&line);
        if let Some(k) = kw.strip_suffix(':') {
            // `Ref: a > b` / `Note: '…'`
            rest = format!(": {rest}");
            kw = k.to_string();
        }
        match kw.to_ascii_lowercase().as_str() {
            "table" => {
                let head = rest.trim_end_matches('{').trim();
                let (head, settings) = take_settings(head);
                let (name, after) = token(&head);
                let alias = after.trim().strip_prefix("as ").map(|a| unquote(a.trim()));
                let mut t = Table {
                    name: table_name(&name),
                    alias,
                    color: setting_value(&settings, "headercolor").map(String::from),
                    note: setting_value(&settings, "note").map(unquote),
                    ..Default::default()
                };
                if !line.trim_end().ends_with('{') && lines.get(i).map(|l| l.1.as_str()) == Some("{") {
                    i += 1;
                }
                let ti = erd.tables.len();
                while i < lines.len() {
                    let (cno, cl) = lines[i].clone();
                    i += 1;
                    if cl.starts_with('}') {
                        break;
                    }
                    let (first, crest) = token(&cl);
                    let lower = first.to_ascii_lowercase();
                    if lower == "indexes" || lower == "indexes{" {
                        skip_block(&mut i, &lines);
                        continue;
                    }
                    let ct = crest.trim_start();
                    if lower == "note:" || (lower == "note" && (ct.starts_with(':') || ct.starts_with('{'))) {
                        let v = crest.trim().trim_start_matches(':').trim();
                        if v.starts_with('{') {
                            if let Some((_, n)) = lines.get(i) {
                                t.note = Some(unquote(n));
                            }
                            skip_block(&mut i, &lines);
                        } else {
                            t.note = Some(unquote(v));
                        }
                        continue;
                    }
                    let (ty_and, settings) = take_settings(&crest);
                    let ty = unquote(&ty_and);
                    if ty.is_empty() {
                        erd.errors.push(format!("line {cno}: column without a type: {cl}"));
                        continue;
                    }
                    let mut c = Column { name: unquote(&first), ty, ..Default::default() };
                    for s in &settings {
                        let l = s.to_ascii_lowercase();
                        match l.as_str() {
                            "pk" | "primary key" => c.pk = true,
                            "not null" => c.not_null = true,
                            "unique" => c.unique = true,
                            "increment" => c.increment = true,
                            _ => {}
                        }
                        if let Some((k, v)) = s.split_once(':') {
                            match k.trim().to_ascii_lowercase().as_str() {
                                "note" => c.note = Some(unquote(v)),
                                "default" => c.default = Some(unquote(v)),
                                "ref" => {
                                    let v = v.trim();
                                    let (op, target) = ["<>", ">", "<", "-"]
                                        .iter()
                                        .find_map(|op| v.strip_prefix(op).map(|t| (op.to_string(), t.trim().to_string())))
                                        .unwrap_or(("-".into(), v.to_string()));
                                    refs.push(PendingRef {
                                        line: cno,
                                        from: format!("{}.{}", t.name, c.name),
                                        op,
                                        to: target,
                                        name: None,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                    t.columns.push(c);
                }
                let _ = ti;
                erd.tables.push(t);
            }
            "ref" => {
                let (name, body) = match rest.split_once(':') {
                    Some((n, b)) if !n.contains('.') => (Some(unquote(n)).filter(|n| !n.is_empty()), b.trim().to_string()),
                    _ => (None, rest.trim_start_matches(':').trim().to_string()),
                };
                let mut specs = vec![];
                if body.ends_with('{') || (body.is_empty() && lines.get(i).map(|l| l.1.as_str()) == Some("{")) {
                    let name = if body.ends_with('{') {
                        Some(unquote(body.trim_end_matches('{').trim())).filter(|n| !n.is_empty()).or(name.clone())
                    } else {
                        name.clone()
                    };
                    while i < lines.len() {
                        let (rno, rl) = lines[i].clone();
                        i += 1;
                        if rl == "{" {
                            continue;
                        }
                        if rl.starts_with('}') {
                            break;
                        }
                        specs.push((rno, rl, name.clone()));
                    }
                } else {
                    specs.push((no, body, name));
                }
                for (rno, spec, name) in specs {
                    let (spec, _) = take_settings(&spec);
                    match ref_parts(&spec) {
                        Some((a, op, b)) => refs.push(PendingRef { line: rno, from: a, op, to: b, name }),
                        None => erd.errors.push(format!("line {rno}: can't read ref: {spec}")),
                    }
                }
            }
            "enum" => {
                let name = table_name(rest.trim_end_matches('{').trim());
                let mut e = Enum { name, values: vec![] };
                while i < lines.len() {
                    let (_, el) = lines[i].clone();
                    i += 1;
                    if el == "{" {
                        continue;
                    }
                    if el.starts_with('}') {
                        break;
                    }
                    let (v, _) = take_settings(&el);
                    e.values.push(unquote(&v));
                }
                erd.enums.push(e);
            }
            "tablegroup" => {
                let head = rest.trim_end_matches('{').trim();
                let (head, settings) = take_settings(head);
                let g = Group { name: unquote(&head), color: setting_value(&settings, "color").map(String::from), tables: vec![] };
                let mut members = vec![];
                while i < lines.len() {
                    let (_, gl) = lines[i].clone();
                    i += 1;
                    if gl == "{" {
                        continue;
                    }
                    if gl.starts_with('}') {
                        break;
                    }
                    let (tok, _) = token(&gl);
                    let lower = tok.to_ascii_lowercase();
                    if lower.starts_with("note") {
                        continue;
                    }
                    members.push(table_name(&tok));
                }
                group_members.push((erd.groups.len(), members));
                erd.groups.push(g);
            }
            "project" => {
                erd.project = Some(unquote(token(rest.trim_end_matches('{')).0.trim_end_matches('{')));
                if line.matches('{').count() > line.matches('}').count() {
                    skip_block(&mut i, &lines);
                }
            }
            "note" | "tablepartial" | "records" => {
                if line.matches('{').count() > line.matches('}').count() {
                    skip_block(&mut i, &lines);
                }
            }
            _ => erd.errors.push(format!("line {no}: unexpected: {line}")),
        }
    }

    for (g, members) in group_members {
        for m in members {
            match erd.table(&m) {
                Some(t) => {
                    erd.tables[t].group = Some(g);
                    erd.groups[g].tables.push(t);
                }
                None => erd.errors.push(format!("table group {}: no table {m}", erd.groups[g].name)),
            }
        }
    }
    for r in refs {
        let end = |erd: &Erd, s: &str| -> Option<(usize, usize)> {
            let s = s.trim();
            // composite (a, b): use the first column
            let s = match (s.find('('), s.rfind(')')) {
                (Some(o), Some(c)) => {
                    let first = s[o + 1..c].split(',').next().unwrap_or("").trim().to_string();
                    format!("{}{}", &s[..o], first)
                }
                _ => s.to_string(),
            };
            let (t, c) = s.rsplit_once('.')?;
            let ti = erd.table(&table_name(t))?;
            let ci = erd.tables[ti].column(&unquote(c))?;
            Some((ti, ci))
        };
        match (end(&erd, &r.from), end(&erd, &r.to)) {
            (Some(from), Some(to)) => {
                let (fc, tc) = cards(&r.op);
                let new = Ref { from, to, from_card: fc, to_card: tc, name: r.name };
                if !erd.refs.contains(&new) {
                    erd.refs.push(new);
                }
            }
            _ => erd.errors.push(format!("line {}: ref to an unknown column: {} {} {}", r.line, r.from, r.op, r.to)),
        }
    }
    erd
}

#[cfg(test)]
mod tests {
    use super::*;

    const EX: &str = r#"
Project shop { database_type: 'PostgreSQL' }
// users and orders
Table users as U [headercolor: #3498DB, note: 'people'] {
  id integer [pk, increment]
  email varchar(255) [not null, unique, note: 'login']
  status user_status
  created_at timestamp [default: `now()`]
  Note: 'All users'
}
Table "sales"."orders" {
  id int [pk]
  user_id int [ref: > U.id, not null]
  /* block
     comment */
  total decimal(10,2)
  Indexes {
    (user_id, id) [unique]
  }
}
Table items {
  order_id int
  sku varchar
  note text [note: '''
    multi
    line''']
}
Ref fk_items: items.order_id > sales.orders.id [delete: cascade]
Ref: items.sku <> U.email
Ref {
  items.sku - items.sku
}
Enum user_status {
  active
  banned [note: 'no']
}
TableGroup sales_tables [color: #F00] {
  sales.orders
  items
}
"#;

    #[test]
    fn parses_example() {
        let e = parse(EX);
        assert!(e.errors.is_empty(), "{:?}", e.errors);
        assert_eq!(e.project.as_deref(), Some("shop"));
        assert_eq!(e.tables.len(), 3);
        let users = &e.tables[0];
        assert_eq!(users.alias.as_deref(), Some("U"));
        assert_eq!(users.color.as_deref(), Some("#3498DB"));
        assert_eq!(users.note.as_deref(), Some("All users"));
        assert!(users.columns[0].pk && users.columns[0].increment);
        assert!(users.columns[1].not_null && users.columns[1].unique);
        assert_eq!(users.columns[3].default.as_deref(), Some("now()"));
        assert_eq!(e.tables[1].name, "sales.orders");
        assert_eq!(e.tables[1].columns.len(), 3);
        assert_eq!(e.tables[1].columns[2].ty, "decimal(10,2)");
        assert_eq!(e.tables[2].columns[2].note.as_deref().map(|n| n.split_whitespace().count()), Some(2));
        assert_eq!(e.refs.len(), 4);
        assert_eq!(e.refs[0].from, (1, 1));
        assert_eq!(e.refs[0].to, (0, 0));
        assert_eq!(e.refs[0].from_card, Card::Many);
        assert_eq!(e.refs[1].name.as_deref(), Some("fk_items"));
        assert_eq!(e.enums[0].values, vec!["active", "banned"]);
        assert_eq!(e.groups[0].tables, vec![1, 2]);
        assert!(e.is_enum("user_status"));
    }
}
