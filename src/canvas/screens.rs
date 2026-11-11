//! Small line formats: `.rv` review findings, `.rows` row counts, `.doc` / `.open` paths.

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub key: String,
    pub file: String,
    pub line: Option<u32>,
    pub severity: String,
    pub message: String,
    pub fix: String,
}

#[derive(Clone, Debug, Default)]
pub struct Review {
    pub title: String,
    pub findings: Vec<Finding>,
}

/// `? title` then `key: file:line | high|medium|low | what | fix`
pub fn parse_review(text: &str) -> Review {
    let mut rv = Review::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(t) = line.strip_prefix('?') {
            rv.title = t.trim().to_string();
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else { continue };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        let parts: Vec<&str> = rest.split('|').map(|p| p.trim()).collect();
        let where_ = parts.first().copied().unwrap_or("");
        let (file, line_no) = match where_.rsplit_once(':') {
            Some((f, l)) if l.chars().all(|c| c.is_ascii_digit()) && !l.is_empty() => (f.to_string(), l.parse().ok()),
            _ => (where_.to_string(), None),
        };
        rv.findings.push(Finding {
            key: key.to_string(),
            file,
            line: line_no,
            severity: parts.get(1).map(|s| s.to_lowercase()).filter(|s| !s.is_empty()).unwrap_or_else(|| "note".into()),
            message: parts.get(2).unwrap_or(&"").to_string(),
            fix: parts.get(3..).map(|p| p.join(" | ")).unwrap_or_default(),
        });
    }
    rv
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub name: String,
    pub rows_in: Option<f64>,
    pub rows_out: Option<f64>,
    pub cols: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Rows {
    pub title: String,
    pub steps: Vec<Step>,
}

/// `? title` then `step | rows_in | rows_out | columns`
pub fn parse_rows(text: &str) -> Rows {
    let mut out = Rows::default();
    let num = |v: Option<&&str>| -> Option<f64> {
        let s: String = v?.chars().filter(|c| !matches!(c, ',' | '_' | ' ')).collect();
        s.parse().ok()
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(t) = line.strip_prefix('?') {
            out.title = t.trim().to_string();
            continue;
        }
        let p: Vec<&str> = line.split('|').map(|x| x.trim()).collect();
        if p[0].is_empty() {
            continue;
        }
        out.steps.push(Step { name: p[0].to_string(), rows_in: num(p.get(1)), rows_out: num(p.get(2)), cols: num(p.get(3)) });
    }
    out
}

/// `.doc` / `.open`: the first line is a path (`path[:line]`).
pub fn parse_path(text: &str) -> (String, Option<u32>) {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    match first.rsplit_once(':') {
        Some((p, l)) if !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()) => (p.to_string(), l.parse().ok()),
        _ => (first.to_string(), None),
    }
}

/// Paragraph blocks of a Markdown file (split on blank lines, fenced code kept whole).
pub fn blocks(md: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur: Vec<&str> = vec![];
    let mut fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            cur.push(line);
            if !fence {
                out.push(cur.join("\n"));
                cur.clear();
            }
            continue;
        }
        if !fence && line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(cur.join("\n"));
                cur.clear();
            }
            continue;
        }
        cur.push(line);
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_and_rows() {
        let rv = parse_review("? Review x\n1: models/a.sql:42 | high | dedup | use argMax | or not\n# c\n2: README.md | low | typo");
        assert_eq!(rv.title, "Review x");
        assert_eq!(rv.findings[0].line, Some(42));
        assert_eq!(rv.findings[0].fix, "use argMax | or not");
        assert_eq!(rv.findings[1].line, None);
        let r = parse_rows("? t\nsrc | 1,000 | 900 | 12\njoin | 900 | 950");
        assert_eq!(r.steps[0].rows_in, Some(1000.0));
        assert_eq!(r.steps[1].cols, None);
        assert_eq!(parse_path("docs/a.md:12\n"), ("docs/a.md".into(), Some(12)));
        assert_eq!(blocks("# T\n\npara\nmore\n\n```\na\n\nb\n```\n").len(), 3);
    }
}
