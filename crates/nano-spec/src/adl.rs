//! Minimal ADL front-end.
//!
//! This is intentionally a small surface syntax over the existing typed
//! [`AnalysisSpec`](crate::AnalysisSpec), not a new executable path. The grammar
//! accepted here is:
//!
//! ```text
//! analysis <name> year <year>;
//! object <name> : <NanoAOD source> { [select] <cut>; ... }
//! object <name> : pair(<object>) { opposite_charge; selection leading_pt; ... }
//! region <name> { [select] <requirement>; ... }
//! define <name> = <expr>;
//! alias <name> = <expr>;
//! output <name>;
//! output <name> = <expr>;
//! histogram <name> { expr = <expr>; bins = <usize>; range = [<lo>, <hi>]; }
//! weight nominal;
//! systematic <name> kind weight up <factor> down <factor>;
//! correction <name> kind scale collection <object> attr <attr> up <factor> down <factor>;
//! ```
//!
//! Pair objects also accept `comb(<object>, 2)`, `same_flavor`,
//! `nearest_mass <quantity>`, `nearest_mass_truncated <quantity>`,
//! `exclude <pair>[, ...]`, and `filter <comparison>`. Expressions, cuts,
//! requirements, and units are then parsed by the same helpers used by the
//! TOML/YAML/JSON path.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    analysis_spec_from_raw, validate_identifier, AnalysisSpec, ParseError, RawAnalysis,
    RawAnalysisSpec, RawCorrection, RawDerivedObject, RawHistogram, RawObject, RawOutput,
    RawRegion, RawSystematic, RawWeight,
};

pub fn parse_adl(input: &str) -> Result<AnalysisSpec, ParseError> {
    Parser::new(input).parse()
}

struct Parser {
    input: String,
    pos: usize,
    analysis: Option<RawAnalysis>,
    objects: BTreeMap<String, RawObject>,
    derived: BTreeMap<String, RawDerivedObject>,
    regions: BTreeMap<String, RawRegion>,
    outputs: Vec<RawOutput>,
    histograms: Vec<RawHistogram>,
    weight: Option<RawWeight>,
    systematic: Vec<RawSystematic>,
    corrections: Vec<RawCorrection>,
    aliases: BTreeMap<String, String>,
    output_names: BTreeSet<String>,
}

impl Parser {
    fn new(input: &str) -> Self {
        Self {
            input: strip_comments(input),
            pos: 0,
            analysis: None,
            objects: BTreeMap::new(),
            derived: BTreeMap::new(),
            regions: BTreeMap::new(),
            outputs: Vec::new(),
            histograms: Vec::new(),
            weight: None,
            systematic: Vec::new(),
            corrections: Vec::new(),
            aliases: BTreeMap::new(),
            output_names: BTreeSet::new(),
        }
    }

    fn parse(mut self) -> Result<AnalysisSpec, ParseError> {
        while self.skip_ws() {
            if self.consume_keyword("analysis") {
                self.parse_analysis()?;
            } else if self.consume_keyword("object") {
                self.parse_object()?;
            } else if self.consume_keyword("region") {
                self.parse_region()?;
            } else if self.consume_keyword("define") || self.consume_keyword("alias") {
                self.parse_alias()?;
            } else if self.consume_keyword("output") {
                self.parse_output()?;
            } else if self.consume_keyword("histogram") {
                self.parse_histogram()?;
            } else if self.consume_keyword("weight") {
                self.parse_weight()?;
            } else if self.consume_keyword("systematic") {
                self.parse_systematic()?;
            } else if self.consume_keyword("correction") {
                self.parse_correction()?;
            } else {
                return self.err("expected `analysis`, `object`, `region`, `define`, `alias`, `output`, `histogram`, `weight`, `systematic`, or `correction`");
            }
        }

        let analysis = self
            .analysis
            .ok_or_else(|| ParseError::InvalidSpec("ADL spec is missing `analysis`".to_string()))?;

        analysis_spec_from_raw(RawAnalysisSpec {
            analysis,
            objects: self.objects,
            derived: self.derived,
            models: Vec::new(),
            regions: self.regions,
            outputs: self.outputs,
            histograms: self.histograms,
            weight: self.weight,
            systematics: Vec::new(),
            systematic: self.systematic,
            corrections: self.corrections,
            channels: Vec::new(),
        })
    }

    fn parse_analysis(&mut self) -> Result<(), ParseError> {
        if self.analysis.is_some() {
            return self.err("duplicate `analysis` declaration");
        }
        let name = self.parse_identifier("analysis name")?;
        self.expect_keyword("year")?;
        let year = self.parse_identifier("analysis year")?;
        self.expect_char(';')?;
        self.analysis = Some(RawAnalysis { name, year });
        Ok(())
    }

    fn parse_object(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("object name")?;
        self.expect_char(':')?;
        let source = self.read_until_char('{')?.trim().to_string();
        if source.is_empty() {
            return self.err("object declaration is missing a source after `:`");
        }
        let body = self.parse_block()?;

        if let Some(object) = parse_pair_source(&source)? {
            let raw = self.parse_pair_object(&name, object, &body)?;
            self.insert_derived(name, raw)
        } else {
            validate_identifier(&source, "object source")?;
            let cuts = block_statements(&body)
                .into_iter()
                .flat_map(expand_select_statement)
                .map(|stmt| self.expand_aliases(stmt.trim()))
                .collect::<Vec<_>>();
            self.insert_object(name, RawObject { source, cuts })
        }
    }

    fn parse_pair_object(
        &self,
        name: &str,
        object: String,
        body: &str,
    ) -> Result<RawDerivedObject, ParseError> {
        let mut constraints = Vec::new();
        let mut filters = Vec::new();
        let mut selection = None;
        let mut target = None;
        let mut exclude = Vec::new();

        for stmt in block_statements(body) {
            let stmt = stmt.trim();
            if stmt.eq_ignore_ascii_case("opposite_charge") || stmt.eq_ignore_ascii_case("ossf") {
                constraints.push("opposite_charge".to_string());
                if stmt.eq_ignore_ascii_case("ossf") {
                    constraints.push("same_flavor".to_string());
                }
            } else if stmt.eq_ignore_ascii_case("same_flavor") {
                constraints.push("same_flavor".to_string());
            } else if stmt.eq_ignore_ascii_case("leading_pt") {
                selection = Some("leading_pt".to_string());
            } else if let Some(value) = stmt.strip_prefix("selection ") {
                selection = Some(value.trim().to_string());
            } else if let Some(value) = stmt.strip_prefix("nearest_mass_truncated ") {
                selection = Some("nearest_mass_truncated".to_string());
                target = Some(value.trim().to_string());
            } else if let Some(value) = stmt
                .strip_prefix("nearest_mass ")
                .or_else(|| stmt.strip_prefix("nearest "))
            {
                selection = Some("nearest_mass".to_string());
                target = Some(value.trim().to_string());
            } else if let Some(value) = stmt.strip_prefix("exclude ") {
                for item in value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                {
                    validate_identifier(item, "pair exclude")?;
                    exclude.push(item.to_string());
                }
            } else if let Some(value) = stmt
                .strip_prefix("filter ")
                .or_else(|| stmt.strip_prefix("select "))
            {
                filters.push(self.expand_aliases(value.trim()));
            } else if stmt.is_empty() {
            } else {
                return Err(ParseError::InvalidSpec(format!(
                    "failed to parse ADL derived pair `{name}`: unsupported statement `{stmt}`"
                )));
            }
        }

        Ok(RawDerivedObject {
            kind: "pair".to_string(),
            object: Some(object),
            items: Vec::new(),
            constraints,
            filters,
            selection,
            target,
            exclude,
        })
    }

    fn parse_region(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("region name")?;
        let body = self.parse_block()?;
        let require = block_statements(&body)
            .into_iter()
            .flat_map(expand_select_statement)
            .map(|stmt| self.expand_aliases(stmt.trim()))
            .collect::<Vec<_>>();
        if self
            .regions
            .insert(name.clone(), RawRegion { require })
            .is_some()
        {
            return Err(ParseError::InvalidSpec(format!(
                "failed to parse ADL: duplicate region `{name}`"
            )));
        }
        Ok(())
    }

    fn parse_alias(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("alias name")?;
        self.expect_char('=')?;
        let expr = self.read_until_semicolon()?.trim().to_string();
        self.expect_char(';')?;
        if self.aliases.contains_key(&name) {
            return Err(ParseError::InvalidSpec(format!(
                "failed to parse ADL: duplicate alias `{name}`"
            )));
        }
        let expr = self.expand_aliases(&expr);
        self.aliases.insert(name, expr);
        Ok(())
    }

    fn parse_output(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("output name")?;
        let expr = if self.consume_char('=') {
            let expr = self.read_until_semicolon()?.trim().to_string();
            self.expect_char(';')?;
            self.expand_aliases(&expr)
        } else {
            self.expect_char(';')?;
            self.aliases.get(&name).cloned().ok_or_else(|| {
                ParseError::InvalidSpec(format!(
                    "failed to parse ADL output `{name}`: undefined alias `{name}`"
                ))
            })?
        };
        self.push_output(name, expr)
    }

    fn parse_histogram(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("histogram name")?;
        let body = self.parse_block()?;
        let mut expr = None;
        let mut bins = None;
        let mut range = None;

        for stmt in block_statements(&body) {
            let Some((key, value)) = stmt.split_once('=') else {
                return Err(ParseError::InvalidSpec(format!(
                    "failed to parse ADL histogram `{name}`: expected `key = value` statement"
                )));
            };
            match key.trim() {
                "expr" => expr = Some(self.expand_aliases(value.trim())),
                "bins" => {
                    bins = Some(value.trim().parse::<usize>().map_err(|_| {
                        ParseError::InvalidSpec(format!(
                            "failed to parse ADL histogram `{name}`: invalid bins `{}`",
                            value.trim()
                        ))
                    })?);
                }
                "range" => range = Some(parse_range(value.trim(), &name)?),
                other => {
                    return Err(ParseError::InvalidSpec(format!(
                        "failed to parse ADL histogram `{name}`: unsupported key `{other}`"
                    )));
                }
            }
        }

        self.histograms.push(RawHistogram {
            name,
            expr: expr.ok_or_else(|| {
                ParseError::InvalidSpec("failed to parse ADL histogram: missing `expr`".to_string())
            })?,
            bins: bins.ok_or_else(|| {
                ParseError::InvalidSpec("failed to parse ADL histogram: missing `bins`".to_string())
            })?,
            range: range.ok_or_else(|| {
                ParseError::InvalidSpec(
                    "failed to parse ADL histogram: missing `range`".to_string(),
                )
            })?,
        });
        Ok(())
    }

    fn parse_weight(&mut self) -> Result<(), ParseError> {
        if self.weight.is_some() {
            return self.err("duplicate `weight` declaration");
        }
        self.expect_keyword("nominal")?;
        self.expect_char(';')?;
        self.weight = Some(RawWeight {
            nominal: Vec::new(),
        });
        Ok(())
    }

    fn parse_systematic(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("systematic name")?;
        self.expect_keyword("kind")?;
        let kind = self.parse_identifier("systematic kind")?;
        self.expect_keyword("up")?;
        let up = self.parse_number("systematic up factor")?;
        self.expect_keyword("down")?;
        let down = self.parse_number("systematic down factor")?;
        self.expect_char(';')?;
        self.systematic.push(RawSystematic {
            name,
            kind,
            up,
            down,
        });
        Ok(())
    }

    fn parse_correction(&mut self) -> Result<(), ParseError> {
        let name = self.parse_identifier("correction name")?;
        self.expect_keyword("kind")?;
        let kind = self.parse_identifier("correction kind")?;
        self.expect_keyword("collection")?;
        let collection = self.parse_identifier("correction collection")?;
        self.expect_keyword("attr")?;
        let attr = self.parse_identifier("correction attribute")?;
        self.expect_keyword("up")?;
        let up = self.parse_number("correction up factor")?;
        self.expect_keyword("down")?;
        let down = self.parse_number("correction down factor")?;
        self.expect_char(';')?;
        self.corrections.push(RawCorrection {
            name,
            kind,
            collection,
            attr,
            up,
            down,
        });
        Ok(())
    }

    fn insert_object(&mut self, name: String, object: RawObject) -> Result<(), ParseError> {
        if self.derived.contains_key(&name) || self.objects.insert(name.clone(), object).is_some() {
            return Err(ParseError::InvalidSpec(format!(
                "failed to parse ADL: duplicate object `{name}`"
            )));
        }
        Ok(())
    }

    fn insert_derived(
        &mut self,
        name: String,
        derived: RawDerivedObject,
    ) -> Result<(), ParseError> {
        if self.objects.contains_key(&name) || self.derived.insert(name.clone(), derived).is_some()
        {
            return Err(ParseError::InvalidSpec(format!(
                "failed to parse ADL: duplicate object `{name}`"
            )));
        }
        Ok(())
    }

    fn push_output(&mut self, name: String, expr: String) -> Result<(), ParseError> {
        if !self.output_names.insert(name.clone()) {
            return Err(ParseError::InvalidSpec(format!(
                "failed to parse ADL: duplicate output `{name}`"
            )));
        }
        self.outputs.push(RawOutput { name, expr });
        Ok(())
    }

    fn expand_aliases(&self, expr: &str) -> String {
        expand_aliases(expr, &self.aliases)
    }

    fn skip_ws(&mut self) -> bool {
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        self.pos < self.input.len()
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        let rest = &self.input[self.pos..];
        if !rest.starts_with(keyword) {
            return false;
        }
        let after = self.pos + keyword.len();
        if self
            .input
            .get(after..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(is_identifier_continue)
        {
            return false;
        }
        self.pos = after;
        true
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), ParseError> {
        self.skip_ws();
        if self.consume_keyword(keyword) {
            Ok(())
        } else {
            self.err(&format!("expected `{keyword}`"))
        }
    }

    fn parse_identifier(&mut self, context: &str) -> Result<String, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let Some(first) = self.peek_char() else {
            return self.err(&format!("expected {context}"));
        };
        if !is_identifier_start(first) {
            return self.err(&format!("expected {context}"));
        }
        self.pos += first.len_utf8();
        while let Some(ch) = self.peek_char() {
            if is_identifier_continue(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        let ident = self.input[start..self.pos].to_string();
        validate_identifier(&ident, context)?;
        Ok(ident)
    }

    fn parse_number(&mut self, context: &str) -> Result<f64, ParseError> {
        self.skip_ws();
        let start = self.pos;
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || ch == ';' {
                break;
            }
            self.pos += ch.len_utf8();
        }
        if start == self.pos {
            return self.err(&format!("expected {context}"));
        }
        let token = &self.input[start..self.pos];
        token.parse::<f64>().map_err(|_| {
            ParseError::InvalidSpec(format!(
                "failed to parse ADL at line {}: invalid {context} `{token}`",
                self.line()
            ))
        })
    }

    fn expect_char(&mut self, expected: char) -> Result<(), ParseError> {
        self.skip_ws();
        if self.consume_char(expected) {
            Ok(())
        } else {
            self.err(&format!("expected `{expected}`"))
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
        self.skip_ws();
        if self.peek_char() == Some(expected) {
            self.pos += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn read_until_char(&mut self, target: char) -> Result<&str, ParseError> {
        let start = self.pos;
        let mut depth = 0_i32;
        while let Some(ch) = self.peek_char() {
            match ch {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ if ch == target && depth == 0 => return Ok(&self.input[start..self.pos]),
                _ => {}
            }
            self.pos += ch.len_utf8();
        }
        self.err(&format!("expected `{target}`"))
    }

    fn read_until_semicolon(&mut self) -> Result<&str, ParseError> {
        self.read_until_char(';')
    }

    fn parse_block(&mut self) -> Result<String, ParseError> {
        self.expect_char('{')?;
        let start = self.pos;
        let mut depth = 1_i32;
        while let Some(ch) = self.peek_char() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let body = self.input[start..self.pos].to_string();
                        self.pos += ch.len_utf8();
                        return Ok(body);
                    }
                }
                _ => {}
            }
            self.pos += ch.len_utf8();
        }
        self.err("unterminated block")
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn err<T>(&self, message: &str) -> Result<T, ParseError> {
        Err(ParseError::InvalidSpec(format!(
            "failed to parse ADL at line {}: {message}",
            self.line()
        )))
    }

    fn line(&self) -> usize {
        self.input[..self.pos]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1
    }
}

fn strip_comments(input: &str) -> String {
    let mut stripped = String::with_capacity(input.len());
    for line in input.lines() {
        let hash = line.find('#');
        let slash = line.find("//");
        let end = match (hash, slash) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => line.len(),
        };
        stripped.push_str(&line[..end]);
        stripped.push('\n');
    }
    stripped
}

fn parse_pair_source(source: &str) -> Result<Option<String>, ParseError> {
    if let Some(inner) = source
        .strip_prefix("pair(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let object = inner.trim();
        validate_identifier(object, source)?;
        return Ok(Some(object.to_string()));
    }

    if let Some(inner) = source
        .strip_prefix("comb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let args = split_args(inner);
        if args.len() != 2 || args[1].trim() != "2" {
            return Err(ParseError::InvalidSpec(format!(
                "failed to parse ADL source `{source}`: only comb(object, 2) is supported"
            )));
        }
        let object = args[0].trim();
        validate_identifier(object, source)?;
        return Ok(Some(object.to_string()));
    }

    Ok(None)
}

fn block_statements(body: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, ch) in body.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ';' if depth == 0 => {
                let statement = body[start..index].trim();
                if !statement.is_empty() {
                    statements.push(statement);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        statements.push(tail);
    }
    statements
}

fn expand_select_statement(statement: &str) -> Vec<&str> {
    let statement = statement
        .strip_prefix("select ")
        .unwrap_or(statement)
        .trim();
    split_top_level_and(statement)
}

fn split_top_level_and(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            _ if depth == 0 && input[index..].starts_with(" and ") => {
                parts.push(input[start..index].trim());
                index += " and ".len();
                start = index;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    parts.push(input[start..].trim());
    parts
}

fn split_args(input: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(input[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    args.push(input[start..].trim());
    args
}

fn parse_range(input: &str, name: &str) -> Result<[f64; 2], ParseError> {
    let inner = input
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| {
            ParseError::InvalidSpec(format!(
                "failed to parse ADL histogram `{name}`: range must be [lo, hi]"
            ))
        })?;
    let args = split_args(inner);
    if args.len() != 2 {
        return Err(ParseError::InvalidSpec(format!(
            "failed to parse ADL histogram `{name}`: range must have two values"
        )));
    }
    Ok([
        args[0].parse::<f64>().map_err(|_| {
            ParseError::InvalidSpec(format!(
                "failed to parse ADL histogram `{name}`: invalid range bound `{}`",
                args[0]
            ))
        })?,
        args[1].parse::<f64>().map_err(|_| {
            ParseError::InvalidSpec(format!(
                "failed to parse ADL histogram `{name}`: invalid range bound `{}`",
                args[1]
            ))
        })?,
    ])
}

fn expand_aliases(expr: &str, aliases: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(expr.len());
    let mut index = 0;
    while index < expr.len() {
        let ch = expr[index..].chars().next().expect("valid char index");
        if is_identifier_start(ch) {
            let start = index;
            index += ch.len_utf8();
            while index < expr.len() {
                let ch = expr[index..].chars().next().expect("valid char index");
                if is_identifier_continue(ch) {
                    index += ch.len_utf8();
                } else {
                    break;
                }
            }
            let ident = &expr[start..index];
            let next = expr[index..].chars().find(|ch| !ch.is_whitespace());
            if next != Some('(') {
                if let Some(alias) = aliases.get(ident) {
                    output.push('(');
                    output.push_str(alias);
                    output.push(')');
                    continue;
                }
            }
            output.push_str(ident);
        } else {
            output.push(ch);
            index += ch.len_utf8();
        }
    }
    output
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}
