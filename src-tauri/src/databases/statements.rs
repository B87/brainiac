//! Finding statements in a tab's text (SPEC.md, Databases: Running): a small
//! lexer that knows where a `;` ends a statement and where it is inside a
//! string, a quoted name, a comment, a dollar-quoted body, or a trigger's
//! `BEGIN … END`.
//!
//! Offsets here are byte offsets into the text. The window counts in UTF-16
//! code units, so `utf16_to_byte` and `byte_to_utf16` convert at the edges.

use std::ops::Range;

use crate::models::DbKind;

/// The statements in `text`, without surrounding whitespace, each including
/// its closing `;` when it has one. Text that is only whitespace and
/// comments is not a statement.
pub fn split(text: &str, dialect: DbKind) -> Vec<Range<usize>> {
    let mut lexer = Lexer::new(text, dialect);
    let mut out = Vec::new();
    let mut start = 0;
    let mut has_code = false;
    // `BEGIN … END` nesting in a trigger body (SQLite) or `BEGIN ATOMIC` (PostgreSQL).
    let mut depth = 0usize;
    let mut words: Vec<String> = Vec::new();
    while let Some(token) = lexer.next() {
        match token.kind {
            Token::Space | Token::Comment => continue,
            Token::Semicolon if depth == 0 => {
                if has_code {
                    out.push(trimmed(text, start, token.end));
                }
                start = token.end;
                has_code = false;
                words.clear();
                continue;
            }
            Token::Word => {
                let word = text[token.start..token.end].to_ascii_uppercase();
                if opens_block(&words, &word, dialect, depth) || (depth > 0 && word == "CASE") {
                    depth += 1;
                } else if depth > 0 && word == "END" {
                    depth -= 1;
                }
                if words.len() < 8 {
                    words.push(word);
                }
            }
            _ => {}
        }
        if !has_code {
            // Leading comments and blank lines are not part of the statement.
            start = token.start;
            has_code = true;
        }
    }
    if has_code {
        out.push(trimmed(text, start, text.len()));
    }
    out.retain(|r| !r.is_empty());
    out
}

/// Whether `word` opens a block whose `;`s do not end the statement: `BEGIN`
/// in `CREATE TRIGGER` (SQLite), or `BEGIN ATOMIC` in a function body
/// (PostgreSQL 14 and later; the `ATOMIC` is seen next, so `BEGIN` counts here
/// only when the statement creates a function or procedure).
fn opens_block(words: &[String], word: &str, dialect: DbKind, depth: usize) -> bool {
    if word != "BEGIN" {
        return false;
    }
    let creates = |what: &[&str]| {
        words.first().is_some_and(|w| w == "CREATE")
            && words.iter().take(6).any(|w| what.contains(&w.as_str()))
    };
    match dialect {
        DbKind::Sqlite => depth > 0 || creates(&["TRIGGER"]),
        DbKind::Postgres => depth > 0 || creates(&["FUNCTION", "PROCEDURE"]),
    }
}

fn trimmed(text: &str, start: usize, end: usize) -> Range<usize> {
    let slice = &text[start..end];
    let lead = slice.len() - slice.trim_start().len();
    let trail = slice.len() - slice.trim_end().len();
    let (s, e) = (start + lead, end - trail);
    if s >= e {
        s..s
    } else {
        s..e
    }
}

/// The statement to run for a cursor at `cursor` (bytes): the one containing
/// it, or else the last one before it, so a cursor after a statement's `;`
/// runs that statement; before the first, the first.
pub fn statement_at(statements: &[Range<usize>], cursor: usize) -> Option<Range<usize>> {
    statements
        .iter()
        .rev()
        .find(|r| r.start <= cursor)
        .or_else(|| statements.first())
        .cloned()
}

/// The statement without its closing `;` and the space before it, as sent to
/// the database (PostgreSQL's extended protocol takes one statement).
pub fn without_semicolon(statement: &str) -> &str {
    statement.strip_suffix(';').unwrap_or(statement).trim_end()
}

/// A byte offset for a UTF-16 offset from the window, clamped to the text and
/// moved back to the start of a character.
pub fn utf16_to_byte(text: &str, offset: u32) -> usize {
    let mut units = 0u32;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += ch.len_utf16() as u32;
    }
    text.len()
}

/// The UTF-16 offset of a byte offset (which must be on a character boundary).
pub fn byte_to_utf16(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// The byte offset of a 1-based character position in `text`, as PostgreSQL
/// reports where an error is.
pub fn char_position_to_byte(text: &str, position: u32) -> usize {
    text.char_indices()
        .nth(position.saturating_sub(1) as usize)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// The statement's first keyword, upper case, such as `SELECT`.
pub fn first_keyword(statement: &str) -> String {
    let mut lexer = Lexer::new(statement, DbKind::Postgres);
    while let Some(token) = lexer.next() {
        match token.kind {
            Token::Space | Token::Comment => continue,
            Token::Word => return statement[token.start..token.end].to_ascii_uppercase(),
            _ => return String::new(),
        }
    }
    String::new()
}

/// The `:name` parameters in a statement, each name once, in order of
/// first use. A colon inside a string, a quoted name, a comment, or a
/// `::type` cast is not a parameter.
pub fn parameters(sql: &str, dialect: DbKind) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut lexer = Lexer::new(sql, dialect);
    while let Some(token) = lexer.next() {
        if token.kind == Token::Param {
            let name = &sql[token.start + 1..token.end];
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// A statement with each `:name` replaced by `$1`, `$2`, … for PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub sql: String,
    /// The names in the order of their numbers.
    pub names: Vec<String>,
    /// Each replacement: start and length in `sql`, start and length in the original.
    spans: Vec<(usize, usize, usize, usize)>,
}

impl Rewrite {
    /// The byte offset in the original statement of a byte offset in `sql`.
    pub fn original(&self, at: usize) -> usize {
        let mut shift: isize = 0;
        for &(new_start, new_len, old_start, old_len) in &self.spans {
            if at < new_start {
                break;
            }
            if at < new_start + new_len {
                return old_start;
            }
            shift = (old_start + old_len) as isize - (new_start + new_len) as isize;
        }
        (at as isize + shift).max(0) as usize
    }
}

pub fn numbered(sql: &str) -> Rewrite {
    let mut out = String::with_capacity(sql.len());
    let mut names: Vec<String> = Vec::new();
    let mut spans = Vec::new();
    let mut last = 0;
    let mut lexer = Lexer::new(sql, DbKind::Postgres);
    while let Some(token) = lexer.next() {
        if token.kind != Token::Param {
            continue;
        }
        let name = &sql[token.start + 1..token.end];
        let index = match names.iter().position(|n| n == name) {
            Some(i) => i,
            None => {
                names.push(name.to_string());
                names.len() - 1
            }
        };
        out.push_str(&sql[last..token.start]);
        let placeholder = format!("${}", index + 1);
        spans.push((
            out.len(),
            placeholder.len(),
            token.start,
            token.end - token.start,
        ));
        out.push_str(&placeholder);
        last = token.end;
    }
    out.push_str(&sql[last..]);
    Rewrite {
        sql: out,
        names,
        spans,
    }
}

/// What a statement does to a transaction, from its first keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Begin,
    /// `COMMIT`, `END`, `ROLLBACK`, `ABORT`: the transaction ends.
    End,
    /// `ROLLBACK TO SAVEPOINT`: the transaction continues, no longer failed.
    RollbackTo,
    None,
}

pub fn transaction_control(sql: &str) -> Control {
    let mut words = Vec::new();
    let mut lexer = Lexer::new(sql, DbKind::Postgres);
    while let Some(token) = lexer.next() {
        match token.kind {
            Token::Space | Token::Comment => continue,
            Token::Word if words.len() < 3 => {
                words.push(sql[token.start..token.end].to_ascii_uppercase())
            }
            _ => break,
        }
    }
    let word = |i: usize| words.get(i).map(String::as_str);
    // `ROLLBACK [WORK | TRANSACTION] TO [SAVEPOINT] name`.
    let to = word(1) == Some("TO")
        || (matches!(word(1), Some("WORK" | "TRANSACTION")) && word(2) == Some("TO"));
    match (word(0), word(1)) {
        (Some("BEGIN" | "START"), _) => Control::Begin,
        (Some("ROLLBACK" | "ABORT"), _) if to => Control::RollbackTo,
        (Some("COMMIT" | "END" | "ROLLBACK" | "ABORT"), _) => Control::End,
        // The prepared transaction leaves the session, which has none open after it.
        (Some("PREPARE"), Some("TRANSACTION")) => Control::End,
        _ => Control::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Space,
    Comment,
    Word,
    /// Strings, quoted names, numbers, operators: anything else that is code.
    Code,
    Semicolon,
    /// `:name`, a parameter of a saved query.
    Param,
}

struct Lexed {
    kind: Token,
    start: usize,
    end: usize,
}

struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    dialect: DbKind,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str, dialect: DbKind) -> Self {
        Lexer {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            dialect,
        }
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    fn next(&mut self) -> Option<Lexed> {
        let start = self.pos;
        let b = self.peek(0)?;
        let kind = match b {
            b';' => {
                self.pos += 1;
                Token::Semicolon
            }
            b if b.is_ascii_whitespace() => {
                while self.peek(0).is_some_and(|b| b.is_ascii_whitespace()) {
                    self.pos += 1;
                }
                Token::Space
            }
            b'-' if self.peek(1) == Some(b'-') => {
                while self.peek(0).is_some_and(|b| b != b'\n') {
                    self.pos += 1;
                }
                Token::Comment
            }
            b'/' if self.peek(1) == Some(b'*') => {
                self.block_comment();
                Token::Comment
            }
            b'\'' => {
                self.pos += 1;
                self.quoted(b'\'', false);
                Token::Code
            }
            b'E' | b'e'
                if self.peek(1) == Some(b'\'')
                    && self.dialect == DbKind::Postgres
                    && !self.after_word_char(start) =>
            {
                self.pos += 2;
                self.quoted(b'\'', true);
                Token::Code
            }
            b'"' => {
                self.pos += 1;
                self.quoted(b'"', false);
                Token::Code
            }
            b'`' if self.dialect == DbKind::Sqlite => {
                self.pos += 1;
                self.quoted(b'`', false);
                Token::Code
            }
            b'[' if self.dialect == DbKind::Sqlite => {
                while self.peek(0).is_some_and(|b| b != b']') {
                    self.pos += 1;
                }
                self.pos = (self.pos + 1).min(self.bytes.len());
                Token::Code
            }
            b'$' if self.dialect == DbKind::Postgres => {
                if let Some(tag_end) = self.dollar_tag() {
                    let tag = &self.text[start..tag_end];
                    self.pos = tag_end;
                    match self.text[self.pos..].find(tag) {
                        Some(i) => self.pos += i + tag.len(),
                        None => self.pos = self.bytes.len(),
                    }
                } else {
                    // `$1`, a parameter.
                    self.pos += 1;
                    while self.peek(0).is_some_and(|b| b.is_ascii_digit()) {
                        self.pos += 1;
                    }
                }
                Token::Code
            }
            // `:name`, but not the second colon of a `::type` cast or `:=`,
            // nor the bound of an array slice such as `arr[lo:hi]`, where the
            // colon follows a name, a number, or a closing bracket.
            b':' if self.peek(1).is_some_and(|b| is_word_start(b) && b < 0x80)
                && !(start > 0
                    && matches!(self.bytes[start - 1], b':' | b']' | b')')
                        | is_word_char(self.bytes[start - 1])) =>
            {
                self.pos += 1;
                while self
                    .peek(0)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    self.pos += 1;
                }
                Token::Param
            }
            b if is_word_start(b) => {
                while self.peek(0).is_some_and(is_word_char) {
                    self.pos += 1;
                }
                Token::Word
            }
            _ => {
                // One character, whole: never split a multi-byte one.
                let ch = self.text[self.pos..]
                    .chars()
                    .next()
                    .expect("not at the end");
                self.pos += ch.len_utf8();
                Token::Code
            }
        };
        Some(Lexed {
            kind,
            start,
            end: self.pos,
        })
    }

    fn after_word_char(&self, at: usize) -> bool {
        at > 0 && is_word_char(self.bytes[at - 1])
    }

    /// A string or quoted name up to its closing quote; a doubled quote is
    /// one quote, and in `E''` strings a backslash escapes the next character.
    fn quoted(&mut self, quote: u8, backslash: bool) {
        while let Some(b) = self.peek(0) {
            if backslash && b == b'\\' {
                self.pos = (self.pos + 2).min(self.bytes.len());
                continue;
            }
            self.pos += 1;
            if b == quote {
                if self.peek(0) == Some(quote) {
                    self.pos += 1;
                } else {
                    return;
                }
            }
        }
    }

    /// `/* … */`, which PostgreSQL nests and SQLite does not.
    fn block_comment(&mut self) {
        self.pos += 2;
        let mut depth = 1;
        while let Some(b) = self.peek(0) {
            if b == b'*' && self.peek(1) == Some(b'/') {
                self.pos += 2;
                depth -= 1;
                if depth == 0 {
                    return;
                }
            } else if b == b'/' && self.peek(1) == Some(b'*') && self.dialect == DbKind::Postgres {
                self.pos += 2;
                depth += 1;
            } else {
                self.pos += 1;
            }
        }
    }

    /// The end of a dollar-quote tag such as `$$` or `$body$` starting here.
    fn dollar_tag(&self) -> Option<usize> {
        let mut i = self.pos + 1;
        if self.bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            return None;
        }
        while let Some(&b) = self.bytes.get(i) {
            if b == b'$' {
                return Some(i + 1);
            }
            if !is_word_char(b) {
                return None;
            }
            i += 1;
        }
        None
    }
}

fn is_word_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(text: &str, dialect: DbKind) -> Vec<&str> {
        split(text, dialect).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn splits_on_semicolons_outside_strings_names_and_comments() {
        let cases: &[(&str, &[&str])] = &[
            ("select 1; select 2", &["select 1;", "select 2"]),
            ("  select 1 ;\n\n", &["select 1 ;"]),
            (
                "select ';' as s; select 2;",
                &["select ';' as s;", "select 2;"],
            ),
            ("select 'it''s; fine'; x", &["select 'it''s; fine';", "x"]),
            (
                r#"select "a;b" from t; y"#,
                &[r#"select "a;b" from t;"#, "y"],
            ),
            (
                "select 1 -- not; here\n; z",
                &["select 1 -- not; here\n;", "z"],
            ),
            ("-- only a comment\n", &[]),
            ("/* lead */ select 1;", &["select 1;"]),
            (";;", &[]),
            (
                "select 1::int; select ':x'",
                &["select 1::int;", "select ':x'"],
            ),
            ("select 'é;ü'; select 'ß'", &["select 'é;ü';", "select 'ß'"]),
        ];
        for (text, expected) in cases {
            assert_eq!(parts(text, DbKind::Postgres), *expected, "{text}");
        }
    }

    #[test]
    fn knows_postgres_escapes_dollar_quotes_and_nested_comments() {
        let cases: &[(&str, &[&str])] = &[
            (r"select E'a\';b'; z", &[r"select E'a\';b';", "z"]),
            // Outside E'' a backslash is ordinary, so the string ends at `\'`.
            (r"select 'a\'; z", &[r"select 'a\';", "z"]),
            (
                "create function f() returns int as $$ select 1; $$ language sql; z",
                &["create function f() returns int as $$ select 1; $$ language sql;", "z"],
            ),
            (
                "do $body$ begin perform 1; end $body$; z",
                &["do $body$ begin perform 1; end $body$;", "z"],
            ),
            ("select $1; z", &["select $1;", "z"]),
            ("/* a /* b; */ c; */ select 1; z", &["select 1;", "z"]),
            ("select e'x'; z", &["select e'x';", "z"]),
            // `name'…'` is an identifier then a string, not an E string.
            ("select type'x'; z", &["select type'x';", "z"]),
            (
                "create function f() returns int language sql begin atomic select 1; select 2; end; z",
                &[
                    "create function f() returns int language sql begin atomic select 1; select 2; end;",
                    "z",
                ],
            ),
            ("begin; select 1; commit;", &["begin;", "select 1;", "commit;"]),
        ];
        for (text, expected) in cases {
            assert_eq!(parts(text, DbKind::Postgres), *expected, "{text}");
        }
    }

    #[test]
    fn knows_sqlite_quotes_and_trigger_bodies() {
        let cases: &[(&str, &[&str])] = &[
            ("select [a;b] from t; z", &["select [a;b] from t;", "z"]),
            ("select `a;b` from t; z", &["select `a;b` from t;", "z"]),
            // SQLite does not nest comments: the first `*/` ends it.
            ("/* a /* b */ select 1; z", &["select 1;", "z"]),
            (
                "create trigger t after insert on a begin update b set n = n + 1; insert into c values (case when 1 then 2 end); end; z",
                &[
                    "create trigger t after insert on a begin update b set n = n + 1; insert into c values (case when 1 then 2 end); end;",
                    "z",
                ],
            ),
            ("begin; select 1; end;", &["begin;", "select 1;", "end;"]),
        ];
        for (text, expected) in cases {
            assert_eq!(parts(text, DbKind::Sqlite), *expected, "{text}");
        }
    }

    #[test]
    fn an_unclosed_string_runs_to_the_end() {
        assert_eq!(
            parts("select 'abc; def", DbKind::Postgres),
            ["select 'abc; def"]
        );
        assert_eq!(
            parts("select $$ abc; def", DbKind::Postgres),
            ["select $$ abc; def"]
        );
    }

    #[test]
    fn the_cursor_picks_the_statement_it_is_in_or_after() {
        let text = "select 1;\nselect 2;\n\nselect 3";
        let statements = split(text, DbKind::Postgres);
        let at = |c: usize| &text[statement_at(&statements, c).unwrap()];
        assert_eq!(at(0), "select 1;");
        assert_eq!(at(9), "select 1;");
        assert_eq!(at(10), "select 2;");
        assert_eq!(at(20), "select 2;");
        assert_eq!(at(text.len()), "select 3");
        assert!(statement_at(&[], 0).is_none());
    }

    #[test]
    fn offsets_convert_between_bytes_and_utf16() {
        let text = "é😀a";
        assert_eq!(byte_to_utf16(text, 0), 0);
        assert_eq!(byte_to_utf16(text, 2), 1);
        assert_eq!(byte_to_utf16(text, 6), 3);
        assert_eq!(byte_to_utf16(text, 7), 4);
        assert_eq!(utf16_to_byte(text, 1), 2);
        assert_eq!(utf16_to_byte(text, 3), 6);
        // Inside the surrogate pair: the start of the emoji.
        assert_eq!(utf16_to_byte(text, 2), 6);
        assert_eq!(utf16_to_byte(text, 99), text.len());
        assert_eq!(char_position_to_byte("éx", 2), 2);
    }

    #[test]
    fn parameters_are_found_outside_strings_comments_and_casts() {
        let sql = "select :days::int, ':no', \"a:b\" -- :nope\n from t where id = :id and d > :days and x := 1 /* :c */";
        assert_eq!(parameters(sql, DbKind::Postgres), ["days", "id"]);
        assert_eq!(
            parameters("select $$ :x $$, arr[1:2]", DbKind::Postgres),
            Vec::<String>::new()
        );
        // Slice bounds are not parameters; a parameter can still be one.
        assert_eq!(
            parameters(
                "select arr[lo:hi], arr[f(x):hi], arr[:n], m[1][2:k] from t where a=:a",
                DbKind::Postgres
            ),
            ["n", "a"]
        );
        let original = "select * from t where a = :alpha and b = :b or zz";
        let rewrite = numbered(original);
        assert_eq!(rewrite.sql, "select * from t where a = $1 and b = $2 or zz");
        assert_eq!(rewrite.names, ["alpha", "b"]);
        // Positions after a replacement move back to the original text.
        let zz = rewrite.sql.find("zz").unwrap();
        assert_eq!(rewrite.original(zz), original.find("zz").unwrap());
        assert_eq!(
            rewrite.original(rewrite.sql.find("$2").unwrap()),
            original.find(":b").unwrap()
        );
        assert_eq!(rewrite.original(3), 3);
        assert_eq!(transaction_control(" begin"), Control::Begin);
        assert_eq!(
            transaction_control("rollback to savepoint a"),
            Control::RollbackTo
        );
        assert_eq!(transaction_control("ROLLBACK"), Control::End);
        assert_eq!(
            transaction_control("rollback transaction to savepoint a"),
            Control::RollbackTo
        );
        assert_eq!(transaction_control("ROLLBACK WORK"), Control::End);
        assert_eq!(transaction_control("prepare transaction 'x'"), Control::End);
        assert_eq!(transaction_control("prepare q as select 1"), Control::None);
        assert_eq!(transaction_control("select 1"), Control::None);
        assert_eq!(
            parameters("select :é", DbKind::Sqlite),
            Vec::<String>::new()
        );
    }

    #[test]
    fn first_keyword_skips_comments() {
        assert_eq!(
            first_keyword("-- hi\n /* x */ Update t set a = 1"),
            "UPDATE"
        );
        assert_eq!(first_keyword("(select 1)"), "");
        assert_eq!(without_semicolon("select 1 ;"), "select 1");
    }
}
