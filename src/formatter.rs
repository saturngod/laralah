//! "Format code" for the scratchpad editor.
//!
//! Laravel Pint is used when it can be found (the project's own `vendor/bin/pint`
//! first, so its `pint.json` applies, then a global Composer install). Without
//! Pint — or when the buffer doesn't parse yet — a built-in, syntax-tolerant
//! re-indenter fixes indentation and whitespace instead.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Which formatter produced a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatterUsed {
    Pint,
    BuiltIn,
}

#[derive(Debug)]
pub struct FormatOutcome {
    pub text: String,
    pub used: FormatterUsed,
    /// Set when Pint was available but couldn't run (e.g. a syntax error), so the
    /// built-in re-indenter was used instead.
    pub pint_error: Option<String>,
}

/// Formats `code`, preferring Pint. `project_dir` is a local Laravel checkout
/// (if any) and `indent` the editor's indentation width for the fallback.
pub fn format_php(
    code: &str,
    project_dir: Option<&str>,
    php_bin: &str,
    indent: usize,
) -> FormatOutcome {
    let mut pint_error = None;
    if let Some(pint) = find_pint(project_dir) {
        match run_pint(code, &pint, project_dir, php_bin) {
            Ok(text) => {
                return FormatOutcome { text, used: FormatterUsed::Pint, pint_error: None };
            }
            Err(err) => pint_error = Some(err),
        }
    }
    FormatOutcome {
        text: reindent_php(code, indent),
        used: FormatterUsed::BuiltIn,
        pint_error,
    }
}

fn find_pint(project_dir: Option<&str>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = project_dir.map(str::trim).filter(|d| !d.is_empty()) {
        candidates.push(Path::new(dir).join("vendor/bin/pint"));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        candidates.push(home.join(".composer/vendor/bin/pint"));
        candidates.push(home.join(".config/composer/vendor/bin/pint"));
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("Composer/vendor/bin/pint"));
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path_var).map(|d| d.join("pint")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Pint only works on files, so the buffer goes through a temp file. Snippets
/// without an opening tag get one added for Pint and stripped again afterwards.
fn run_pint(
    code: &str,
    pint: &Path,
    project_dir: Option<&str>,
    php_bin: &str,
) -> Result<String, String> {
    let has_open_tag = code.trim_start().starts_with("<?");
    let payload = if has_open_tag { code.to_string() } else { format!("<?php\n\n{code}") };

    let mut file = tempfile::Builder::new()
        .prefix("laralah-format-")
        .suffix(".php")
        .tempfile()
        .map_err(|e| format!("Couldn't create a temp file: {e}"))?;
    file.write_all(payload.as_bytes())
        .and_then(|_| file.flush())
        .map_err(|e| format!("Couldn't write a temp file: {e}"))?;

    // Composer's bin proxies and the Pint phar are PHP scripts: run them with the
    // configured PHP so a GUI app's minimal PATH doesn't matter, and silence the
    // deprecation notices older Pint builds print on newer PHP.
    // Only the first bytes matter, and the Pint phar is several MB.
    let is_php_script = std::fs::File::open(pint)
        .and_then(|f| {
            let mut head = Vec::with_capacity(256);
            std::io::Read::take(f, 256).read_to_end(&mut head)?;
            Ok(head)
        })
        .map(|bytes| {
            let head = String::from_utf8_lossy(&bytes);
            head.starts_with("<?php") || head.lines().next().is_some_and(|l| l.contains("php"))
        })
        .unwrap_or(false);
    let mut cmd = if is_php_script {
        let mut c = Command::new(php_bin);
        c.args(["-d", "error_reporting=0", "-d", "display_errors=0"]).arg(pint);
        c
    } else {
        Command::new(pint)
    };
    cmd.args(["--no-interaction", "--no-ansi"])
        .arg(file.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Pint reads `pint.json` from the working directory.
    if let Some(dir) = project_dir.filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }

    let output = cmd.output().map_err(|e| format!("Couldn't run Pint: {e}"))?;
    if !output.status.success() {
        return Err(pint_failure_message(&output.stdout, &output.stderr));
    }
    let formatted = std::fs::read_to_string(file.path())
        .map_err(|e| format!("Couldn't read Pint's output: {e}"))?;
    Ok(if has_open_tag { formatted } else { strip_open_tag(&formatted) })
}

fn pint_failure_message(stdout: &[u8], stderr: &[u8]) -> String {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    if let Some(idx) = combined.find("Parse error:") {
        let msg = combined[idx..].lines().next().unwrap_or("").trim();
        return msg.trim_end_matches('.').to_string();
    }
    combined
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && l.chars().any(char::is_alphanumeric))
        .unwrap_or("Pint failed")
        .to_string()
}

fn strip_open_tag(formatted: &str) -> String {
    let rest = formatted.trim_start();
    let rest = rest.strip_prefix("<?php").unwrap_or(rest);
    rest.trim_start_matches(['\n', '\r', ' ', '\t']).to_string()
}

// ---- Built-in re-indenter ----

/// Where a line starts, lexically.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Lex {
    Code,
    BlockComment,
    SingleQuoted,
    DoubleQuoted,
    Backtick,
    Heredoc(String),
}

/// Re-indents PHP by bracket nesting, without needing the code to parse.
///
/// Each line gets one level per open bracket that a previous line left open (a
/// line that opens several brackets still adds only one level), method-chain and
/// boolean-operator continuation lines get one extra level, and `case` bodies are
/// indented inside `switch`. Trailing whitespace is removed and runs of blank
/// lines collapse to one. Multi-line strings, heredocs, and anything after a `?>`
/// are left exactly as written.
pub fn reindent_php(code: &str, indent: usize) -> String {
    let unit = " ".repeat(indent.clamp(1, 8));
    let had_trailing_newline = code.ends_with('\n');

    // Each open bracket and whether it added an indent level.
    let mut stack: Vec<(char, bool)> = Vec::new();
    // Brace depths (stack lengths) that are inside an active `case` body.
    let mut case_depths: Vec<usize> = Vec::new();
    let mut lex = Lex::Code;
    let mut stopped = false;
    let mut out: Vec<String> = Vec::new();

    for raw in code.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if stopped {
            out.push(raw.to_string());
            continue;
        }

        let starts_in = lex.clone();
        let verbatim = matches!(
            starts_in,
            Lex::SingleQuoted | Lex::DoubleQuoted | Lex::Backtick | Lex::Heredoc(_)
        );
        let trimmed = raw.trim();

        // Leading closers decide this line's own level.
        let mut sim = stack.clone();
        if starts_in == Lex::Code {
            for ch in trimmed.chars() {
                if matches!(ch, '}' | ')' | ']') {
                    sim.pop();
                } else {
                    break;
                }
            }
        }
        let mut level = sim.iter().filter(|(_, c)| *c).count();

        // `case`/`default` labels sit at the switch level; their bodies one deeper.
        let is_case_label = starts_in == Lex::Code && is_case_label(trimmed);
        let brace_depth = sim.len();
        if is_case_label {
            case_depths.retain(|d| *d < brace_depth);
            level += case_depths.len();
            case_depths.push(brace_depth);
        } else if starts_in == Lex::Code {
            level += case_depths.iter().filter(|d| **d <= brace_depth).count();
        }
        if starts_in == Lex::Code && is_continuation(trimmed) {
            level += 1;
        }

        let stack_before = stack.len();
        let (next_lex, hit_close_tag) = scan_line(raw, starts_in.clone(), &mut stack);
        lex = next_lex;
        // Drop case scopes whose switch body just closed.
        case_depths.retain(|d| *d <= stack.len());

        // At most one indent level per line: the innermost bracket it leaves open.
        if stack.len() > stack_before {
            let newest = stack.len() - 1;
            stack[newest].1 = true;
        }

        if verbatim {
            out.push(raw.to_string());
        } else if trimmed.is_empty() {
            out.push(String::new());
        } else if starts_in == Lex::BlockComment {
            let body = if trimmed.starts_with('*') { format!(" {trimmed}") } else { trimmed.to_string() };
            out.push(format!("{}{}", unit.repeat(level), body));
        } else {
            out.push(format!("{}{}", unit.repeat(level), trimmed));
        }
        if hit_close_tag {
            stopped = true;
        }
    }

    // Collapse blank-line runs and trim blank lines at both ends.
    let mut collapsed: Vec<String> = Vec::with_capacity(out.len());
    for line in out {
        if line.is_empty() && collapsed.last().is_none_or(|l: &String| l.is_empty()) {
            continue;
        }
        collapsed.push(line);
    }
    while collapsed.last().is_some_and(|l| l.is_empty()) {
        collapsed.pop();
    }
    let mut result = collapsed.join("\n");
    if had_trailing_newline && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn is_case_label(trimmed: &str) -> bool {
    (trimmed.starts_with("case ") && trimmed.contains(':'))
        || trimmed
            .strip_prefix("default")
            .is_some_and(|rest| rest.trim_start().starts_with(':'))
}

/// A line that continues the previous statement: `->where(...)`, `&& $x`, `?? 0`.
fn is_continuation(trimmed: &str) -> bool {
    const PREFIXES: &[&str] = &["->", "?->", "&&", "||", "??", "?:", "? ", ": ", "and ", "or ", "xor "];
    if PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return true;
    }
    // String concatenation, but not the `...` spread operator.
    trimmed.starts_with('.') && !trimmed.starts_with("..")
}

/// Scans one line from state `lex`, updating the bracket stack. Returns the state
/// the next line starts in and whether a `?>` closing tag was reached.
fn scan_line(line: &str, mut lex: Lex, stack: &mut Vec<(char, bool)>) -> (Lex, bool) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    if let Lex::Heredoc(ref id) = lex {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(id.as_str())
            && !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')
        {
            // The closing marker; anything after it (`;`, `)`) is code.
            i = chars.len() - rest.chars().count();
            lex = Lex::Code;
        } else {
            return (lex, false);
        }
    }

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match lex {
            Lex::BlockComment => {
                if c == '*' && next == Some('/') {
                    lex = Lex::Code;
                    i += 1;
                }
            }
            Lex::SingleQuoted | Lex::DoubleQuoted | Lex::Backtick => {
                let quote = match lex {
                    Lex::SingleQuoted => '\'',
                    Lex::DoubleQuoted => '"',
                    _ => '`',
                };
                if c == '\\' {
                    i += 1;
                } else if c == quote {
                    lex = Lex::Code;
                }
            }
            Lex::Heredoc(_) => unreachable!("heredoc bodies are handled per line"),
            Lex::Code => match c {
                '/' if next == Some('/') => return (Lex::Code, line_has_close_tag(&chars[i..])),
                '#' if next != Some('[') => return (Lex::Code, line_has_close_tag(&chars[i..])),
                '/' if next == Some('*') => {
                    lex = Lex::BlockComment;
                    i += 1;
                }
                '\'' => lex = Lex::SingleQuoted,
                '"' => lex = Lex::DoubleQuoted,
                '`' => lex = Lex::Backtick,
                '?' if next == Some('>') => return (Lex::Code, true),
                '<' if chars[i..].starts_with(&['<', '<', '<']) => {
                    let rest: String = chars[i + 3..].iter().collect();
                    let id: String = rest
                        .trim_start()
                        .trim_start_matches(['\'', '"'])
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !id.is_empty() {
                        return (Lex::Heredoc(id), false);
                    }
                    i += 2;
                }
                '{' | '(' | '[' => stack.push((c, false)),
                '}' | ')' | ']' => {
                    stack.pop();
                }
                _ => {}
            },
        }
        i += 1;
    }
    (lex, false)
}

/// `?>` ends a single-line comment too.
fn line_has_close_tag(rest: &[char]) -> bool {
    rest.windows(2).any(|w| w == ['?', '>'])
}

/// Maps a caret offset in `old` to the same spot in its reformatted `new`.
///
/// Whitespace is ignored. Pint also rewrites tokens (quotes, `array()` → `[]`,
/// trailing commas, import order), so the caret is placed by counting
/// non-whitespace characters from the start when it sits before the first
/// change, and from the end when it sits after the last one; only a caret
/// inside a changed stretch falls back to the count from the start.
pub fn map_offset(old: &str, new: &str, offset: usize) -> usize {
    let mut offset = offset.min(old.len());
    while !old.is_char_boundary(offset) {
        offset -= 1;
    }
    let old_ns: Vec<char> = old.chars().filter(|c| !c.is_whitespace()).collect();
    let new_ns: Vec<(usize, char)> = new
        .char_indices()
        .filter(|(_, c)| !c.is_whitespace())
        .collect();
    // Non-whitespace characters before the caret.
    let before = old[..offset].chars().filter(|c| !c.is_whitespace()).count();
    if before == 0 {
        return 0;
    }

    let common_prefix = old_ns
        .iter()
        .zip(&new_ns)
        .take_while(|(a, (_, b))| *a == b)
        .count();
    let max_suffix = old_ns.len().min(new_ns.len()) - common_prefix;
    let common_suffix = old_ns
        .iter()
        .rev()
        .zip(new_ns.iter().rev())
        .take(max_suffix)
        .take_while(|(a, (_, b))| *a == b)
        .count();

    // How many of `new`'s non-whitespace characters go before the caret.
    let after = old_ns.len() - before;
    let keep = if before > common_prefix && after <= common_suffix {
        new_ns.len() - after
    } else {
        before.min(new_ns.len())
    };
    match keep.checked_sub(1).map(|i| new_ns[i]) {
        Some((pos, ch)) => pos + ch.len_utf8(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reindents_blocks_chains_and_arrays() {
        let input = "use App\\Models\\User;\n\n\n$users = User::query()\n->where('a', 1)\n      ->get();   \nforeach ($users as $u) {\nif ($u->active) {\necho $u->name;\n}\n}\n$data = collect([\n'a' => 1,\n'b' => [\n2,\n],\n]);\n";
        let expected = "use App\\Models\\User;\n\n$users = User::query()\n    ->where('a', 1)\n    ->get();\nforeach ($users as $u) {\n    if ($u->active) {\n        echo $u->name;\n    }\n}\n$data = collect([\n    'a' => 1,\n    'b' => [\n        2,\n    ],\n]);\n";
        assert_eq!(reindent_php(input, 4), expected);
    }

    #[test]
    fn closures_inside_calls_close_on_one_level() {
        let input = "$x = $users->map(function ($u) {\nreturn $u->id;\n})->all();\nfoo(function () {\nbar();\n}, $arg);";
        let expected = "$x = $users->map(function ($u) {\n    return $u->id;\n})->all();\nfoo(function () {\n    bar();\n}, $arg);";
        assert_eq!(reindent_php(input, 4), expected);
    }

    #[test]
    fn leaves_strings_heredocs_and_comments_alone() {
        let input = "$sql = \"select *\n   from users\n  where id = 1\";\n$t = <<<EOT\n  keep {\n    this\nEOT;\nif (true) {\n// a { comment\n/*\n* doc\n*/\n$a = '}';\n}";
        let expected = "$sql = \"select *\n   from users\n  where id = 1\";\n$t = <<<EOT\n  keep {\n    this\nEOT;\nif (true) {\n    // a { comment\n    /*\n     * doc\n     */\n    $a = '}';\n}";
        assert_eq!(reindent_php(input, 4), expected);
    }

    #[test]
    fn indents_switch_cases_and_tolerates_unbalanced_code() {
        let input = "switch ($x) {\ncase 1:\nfoo();\nbreak;\ndefault:\nbar();\n}\n$y = foo(\n1,";
        let expected = "switch ($x) {\n    case 1:\n        foo();\n        break;\n    default:\n        bar();\n}\n$y = foo(\n    1,";
        assert_eq!(reindent_php(input, 4), expected);

        let nested = "switch ($x) {\ncase 1:\nif ($y) {\nfoo();\n}\nbreak;\n}\ndone();";
        let expected = "switch ($x) {\n    case 1:\n        if ($y) {\n            foo();\n        }\n        break;\n}\ndone();";
        assert_eq!(reindent_php(nested, 4), expected);
    }

    #[test]
    fn honours_indent_width_and_attributes() {
        let input = "#[Attr]\nfunction f() {\nreturn 1;\n}";
        assert_eq!(reindent_php(input, 2), "#[Attr]\nfunction f() {\n  return 1;\n}");
    }

    #[test]
    fn maps_caret_across_whitespace_changes() {
        let old = "if(1){\nfoo();\n}";
        let new = "if (1) {\n    foo();\n}";
        let caret = old.find("foo").unwrap() + 3;
        assert_eq!(&new[..map_offset(old, new, caret)], "if (1) {\n    foo");
        assert_eq!(map_offset(old, new, 0), 0);
        assert_eq!(map_offset(old, new, old.len()), new.len());

        // Pint rewrote code before the caret: count from the end instead.
        let old = "$a = array(1, 2);\n$b = \"x\";\nfoo($b);\n";
        let new = "$a = [1, 2];\n$b = 'x';\nfoo($b);\n";
        let caret = old.find("foo(").unwrap() + 3;
        assert_eq!(&new[..map_offset(old, new, caret)], "$a = [1, 2];\n$b = 'x';\nfoo");
        // ...and a caret before the first change still counts from the start.
        assert_eq!(&new[..map_offset(old, new, 2)], "$a");
        assert_eq!(map_offset("x", "", 1), 0);
    }

    /// Runs against a real Pint install when `LARALAH_TEST_PINT_PROJECT` points at a
    /// Laravel project that has `vendor/bin/pint`; skipped otherwise.
    #[test]
    fn formats_with_pint_and_falls_back_on_syntax_errors() {
        let Ok(project) = std::env::var("LARALAH_TEST_PINT_PROJECT") else {
            return;
        };
        let php = crate::runner::resolve_php_binary();
        let out = format_php("$users = User::query()\n->where(\"a\",1)\n->get();", Some(&project), &php, 4);
        assert_eq!(out.used, FormatterUsed::Pint, "{:?}", out.pint_error);
        assert_eq!(out.text, "$users = User::query()\n    ->where('a', 1)\n    ->get();\n");

        let broken = format_php("if (true) {\nfoo(", Some(&project), &php, 4);
        assert_eq!(broken.used, FormatterUsed::BuiltIn);
        assert!(broken.pint_error.as_deref().is_some_and(|e| e.contains("Parse error")), "{:?}", broken.pint_error);
        assert_eq!(broken.text, "if (true) {\n    foo(");
    }

    #[test]
    fn strips_added_open_tag() {
        assert_eq!(strip_open_tag("<?php\n\n$a = 1;\n"), "$a = 1;\n");
    }
}
