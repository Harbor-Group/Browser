#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum RunAt {
    DocumentStart,
    DocumentEnd,
    DocumentIdle,
}

#[derive(Debug, Clone)]
pub struct UserScript {
    pub name: String,
    pub version: Option<String>,
    pub grants: Vec<String>,
    pub matches: Vec<String>,
    pub excludes: Vec<String>,
    pub run_at: RunAt,
    pub source: String,
    pub enabled: bool,
}

impl UserScript {
    /// Parses standard `// ==UserScript==` metadata blocks.
    pub fn parse(source: String, enabled: bool) -> Self {
        let mut name = String::from("Untitled Script");
        let mut version = None;
        let mut grants = Vec::new();
        let mut matches = Vec::new();
        let mut excludes = Vec::new();
        let mut run_at = RunAt::DocumentEnd;
        let mut in_header = false;

        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed == "// ==UserScript==" {
                in_header = true;
                continue;
            }
            if trimmed == "// ==/UserScript==" {
                break;
            }
            if !in_header || !trimmed.starts_with("//") {
                continue;
            }
            let body = trimmed.trim_start_matches("//").trim();
            let Some((tag, value)) = body.split_once(char::is_whitespace) else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match tag {
                "@name" => name = value.to_string(),
                "@version" => version = Some(value.to_string()),
                "@grant" => grants.push(value.to_string()),
                "@match" | "@include" => matches.push(value.to_string()),
                "@exclude" | "@exclude-match" => excludes.push(value.to_string()),
                "@run-at" => {
                    run_at = match value {
                        "document-start" => RunAt::DocumentStart,
                        "document-idle" => RunAt::DocumentIdle,
                        _ => RunAt::DocumentEnd,
                    }
                }
                _ => {}
            }
        }

        Self {
            name,
            version,
            grants,
            matches,
            excludes,
            run_at,
            source,
            enabled,
        }
    }

    pub fn matches_url(&self, url: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let excluded = self.excludes.iter().any(|p| pattern_matches_url(p, url));
        if excluded {
            return false;
        }
        self.matches.iter().any(|p| pattern_matches_url(p, url))
    }

    /// Wraps user script source with GM_* compatibility shims and safe isolation.
    pub fn wrapped_for_injection(&self) -> String {
        let storage_prefix = format!("__harbor_gm:{}:", self.name);
        format!(
            "(function(){{
const unsafeWindow=window;
const GM_info={{script:{{name:{name:?},version:{ver:?}}},scriptHandler:'Harbor'}};
const GM_log=(...a)=>console.log('[UserScript:'+{name:?}+']',...a);
const GM_addStyle=(css)=>{{const s=document.createElement('style');s.textContent=String(css||'');(document.head||document.documentElement)?.appendChild(s);return s;}};
const _p={prefix:?};
const GM_getValue=(k,d)=>{{if(window.__harborIPC?.gmGet)return window.__harborIPC.gmGet({name:?},k,d);try{{const r=localStorage.getItem(_p+k);return r===null?d:JSON.parse(r);}}catch{{return d;}}}};
const GM_setValue=(k,v)=>{{if(window.__harborIPC?.gmSet){{window.__harborIPC.gmSet({name:?},k,v);return;}}try{{localStorage.setItem(_p+k,JSON.stringify(v));}}catch{{}}}};
const GM_deleteValue=(k)=>{{try{{localStorage.removeItem(_p+k);}}catch{{}}}};
const GM_listValues=()=>{{const out=[];try{{for(let i=0;i<localStorage.length;i++){{const k=localStorage.key(i);if(k&&k.startsWith(_p))out.push(k.slice(_p.length));}}}}catch{{}}return out;}};
const GM={{info:GM_info,addStyle:async(c)=>GM_addStyle(c),getValue:async(k,d)=>GM_getValue(k,d),setValue:async(k,v)=>GM_setValue(k,v),deleteValue:async(k)=>GM_deleteValue(k),listValues:async()=>GM_listValues()}};
try{{
{src}
}}catch(e){{console.error('[Harbor UserScript Error]',{name:?},e);}}
}})();",
            name = self.name,
            ver = self.version.as_deref().unwrap_or("1.0"),
            prefix = storage_prefix,
            src = self.source,
        )
    }
}

pub const MAX_SCRIPT_BYTES: usize = 256 * 1024;

pub fn review(source: String) -> Result<UserScript, &'static str> {
    if source.len() > MAX_SCRIPT_BYTES {
        return Err("User scripts must be 256 KB or smaller.");
    }
    let header = source
        .split_once("// ==UserScript==")
        .and_then(|(_, remainder)| remainder.split_once("// ==/UserScript=="));
    if header.is_none() {
        return Err("The file needs a valid UserScript metadata block.");
    }
    let script = UserScript::parse(source, true);
    if script.name == "Untitled Script" || script.matches.is_empty() {
        return Err("The metadata must include a name and a match or include rule.");
    }
    Ok(script)
}

pub(crate) fn pattern_matches_url(pattern: &str, url: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" || pattern == "<all_urls>" {
        return url.starts_with("http://") || url.starts_with("https://");
    }
    // Regex literal: /pattern/ or /pattern/i
    if let Some(rest) = pattern.strip_prefix('/')
        && let Some((regex_body, flags)) = rest.rsplit_once('/')
        && flags.chars().all(|c| matches!(c, 'i' | 'm' | 's' | 'u'))
    {
        return mini_regex_match(regex_body, url, flags.contains('i'));
    }
    // Chrome/Tampermonkey @match syntax: <scheme>://<host><path>
    if let Some((scheme_pat, after_scheme)) = pattern.split_once("://")
        && let Some((url_scheme, url_after_scheme)) = url.split_once("://")
    {
        let scheme_ok = if scheme_pat == "*" {
            url_scheme.eq_ignore_ascii_case("http") || url_scheme.eq_ignore_ascii_case("https")
        } else {
            wildcard_match_ci(scheme_pat, url_scheme)
        };
        if !scheme_ok {
            return false;
        }
        let (pat_host, pat_path) = match after_scheme.find('/') {
            Some(idx) => (&after_scheme[..idx], &after_scheme[idx..]),
            None => (after_scheme, "/*"),
        };
        let (url_host_port, url_path) = match url_after_scheme.find('/') {
            Some(idx) => (&url_after_scheme[..idx], &url_after_scheme[idx..]),
            None => (url_after_scheme, "/"),
        };
        let url_host = url_host_port.split(':').next().unwrap_or(url_host_port);
        let pat_host_only = pat_host.split(':').next().unwrap_or(pat_host);
        let host_ok = if let Some(base) = pat_host_only.strip_prefix("*.") {
            url_host.eq_ignore_ascii_case(base)
                || url_host
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", base.to_ascii_lowercase()))
        } else if pat_host_only == "*" {
            true
        } else {
            wildcard_match_ci(pat_host_only, url_host)
        };
        if !host_ok {
            return false;
        }
        return wildcard_match(pat_path, url_path);
    }
    wildcard_match(pattern, url)
}

fn wildcard_match_ci(pattern: &str, text: &str) -> bool {
    wildcard_match(&pattern.to_ascii_lowercase(), &text.to_ascii_lowercase())
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" || pattern == "<all_urls>" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut remainder = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 && !pattern.starts_with('*') {
            if let Some(rest) = remainder.strip_prefix(part) {
                remainder = rest;
            } else {
                return false;
            }
        } else if let Some(pos) = remainder.find(part) {
            remainder = &remainder[pos + part.len()..];
        } else {
            return false;
        }
    }
    if !pattern.ends_with('*')
        && let Some(last) = parts.last()
    {
        return text.ends_with(last);
    }
    true
}

/// Lightweight, dependency-free matcher for `/.../` UserScript `@include` / `@exclude` regexes.
fn mini_regex_match(pattern: &str, text: &str, case_insensitive: bool) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();
    let anchored_start = p_chars.first() == Some(&'^');
    let start_idx = usize::from(anchored_start);
    let max_start = if anchored_start { 0 } else { t_chars.len() };
    for pos in 0..=max_start {
        if regex_here(&p_chars, start_idx, &t_chars, pos, case_insensitive) {
            return true;
        }
    }
    false
}

fn regex_here(p: &[char], pi: usize, t: &[char], ti: usize, ci: bool) -> bool {
    if pi >= p.len() {
        return true;
    }
    if p[pi] == '$' && pi + 1 == p.len() {
        return ti == t.len();
    }
    // Handle non-capturing or capturing alternation groups: (a|b) or (?:a|b)
    if p[pi] == '('
        && let Some(close) = find_matching_paren(p, pi)
    {
        let inner_start = if pi + 2 < close && p[pi + 1] == '?' && p[pi + 2] == ':' {
            pi + 3
        } else {
            pi + 1
        };
        let (min_rep, max_rep, next_pi) = parse_quantifier(p, close + 1);
        let branches = split_top_level_alts(&p[inner_start..close]);
        return match_group_reps(&branches, p, next_pi, t, ti, min_rep, max_rep, ci, 0);
    }
    let (atom_len, neg_class, class_end) = if p[pi] == '\\' && pi + 1 < p.len() {
        (2, false, pi + 2)
    } else if p[pi] == '[' {
        if let Some(end) = find_char_class_end(p, pi) {
            (end - pi + 1, p.get(pi + 1) == Some(&'^'), end + 1)
        } else {
            (1, false, pi + 1)
        }
    } else {
        (1, false, pi + 1)
    };
    let (min_rep, max_rep, next_pi) = parse_quantifier(p, class_end);
    let atom = &p[pi..pi + atom_len];
    let mut matched_positions = vec![ti];
    let mut cur = ti;
    while matched_positions.len() - 1 < max_rep
        && cur < t.len()
        && atom_matches(atom, neg_class, t[cur], ci)
    {
        cur += 1;
        matched_positions.push(cur);
    }
    // Try longest match first (greedy)
    for count in (min_rep..matched_positions.len()).rev() {
        if regex_here(p, next_pi, t, matched_positions[count], ci) {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn match_group_reps(
    branches: &[&[char]],
    p: &[char],
    next_pi: usize,
    t: &[char],
    ti: usize,
    min_rep: usize,
    max_rep: usize,
    ci: bool,
    depth: usize,
) -> bool {
    if depth >= min_rep && regex_here(p, next_pi, t, ti, ci) {
        return true;
    }
    if depth >= max_rep {
        return false;
    }
    for branch in branches {
        // Match branch prefix against t[ti..]
        for end_ti in ti..=t.len() {
            if branch_matches_exact(branch, t, ti, end_ti, ci)
                && (end_ti > ti || depth + 1 >= min_rep)
                && match_group_reps(
                    branches,
                    p,
                    next_pi,
                    t,
                    end_ti,
                    min_rep,
                    max_rep,
                    ci,
                    depth + 1,
                )
            {
                return true;
            }
        }
    }
    false
}

fn branch_matches_exact(branch: &[char], t: &[char], start: usize, end: usize, ci: bool) -> bool {
    let mut anchored = Vec::with_capacity(branch.len() + 1);
    anchored.extend_from_slice(branch);
    anchored.push('$');
    regex_here(&anchored, 0, &t[..end], start, ci)
}

fn parse_quantifier(p: &[char], idx: usize) -> (usize, usize, usize) {
    if idx >= p.len() {
        return (1, 1, idx);
    }
    match p[idx] {
        '?' => (0, 1, idx + 1),
        '*' => (0, usize::MAX, idx + 1),
        '+' => (1, usize::MAX, idx + 1),
        _ => (1, 1, idx),
    }
}

fn find_matching_paren(p: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < p.len() {
        if p[i] == '\\' {
            i += 2;
            continue;
        }
        if p[i] == '(' {
            depth += 1;
        } else if p[i] == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn split_top_level_alts(slice: &[char]) -> Vec<&[char]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < slice.len() {
        if slice[i] == '\\' {
            i += 2;
            continue;
        }
        match slice[i] {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                out.push(&slice[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&slice[start..]);
    out
}

fn find_char_class_end(p: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < p.len() {
        if p[i] == '\\' {
            i += 2;
            continue;
        }
        if p[i] == ']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn atom_matches(atom: &[char], neg: bool, ch: char, ci: bool) -> bool {
    if atom == ['.'] {
        return true;
    }
    if atom.len() == 2 && atom[0] == '\\' {
        return match atom[1] {
            'd' => ch.is_ascii_digit(),
            'D' => !ch.is_ascii_digit(),
            'w' => ch.is_ascii_alphanumeric() || ch == '_',
            'W' => !(ch.is_ascii_alphanumeric() || ch == '_'),
            's' => ch.is_whitespace(),
            'S' => !ch.is_whitespace(),
            lit => chars_eq(lit, ch, ci),
        };
    }
    if atom.len() >= 2 && atom[0] == '[' && atom[atom.len() - 1] == ']' {
        let inner = &atom[if neg { 2 } else { 1 }..atom.len() - 1];
        let mut hit = false;
        let mut i = 0;
        while i < inner.len() {
            if i + 2 < inner.len() && inner[i + 1] == '-' {
                let lo = inner[i];
                let hi = inner[i + 2];
                let c = if ci { ch.to_ascii_lowercase() } else { ch };
                let l = if ci { lo.to_ascii_lowercase() } else { lo };
                let h = if ci { hi.to_ascii_lowercase() } else { hi };
                if c >= l && c <= h {
                    hit = true;
                    break;
                }
                i += 3;
            } else {
                if chars_eq(inner[i], ch, ci) {
                    hit = true;
                    break;
                }
                i += 1;
            }
        }
        return if neg { !hit } else { hit };
    }
    atom.len() == 1 && chars_eq(atom[0], ch, ci)
}

fn chars_eq(a: char, b: char, ci: bool) -> bool {
    if ci {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::UserScript;

    #[test]
    fn matches_standard_match_include_exclude_and_regex_rules() {
        let script = UserScript::parse(
            "// ==UserScript==\n\
             // @name Test Script\n\
             // @match *://*.example.com/*\n\
             // @include /^https:\\/\\/(www\\.)?youtube\\.com\\/watch/\n\
             // @exclude *://admin.example.com/*\n\
             // ==/UserScript=="
                .into(),
            true,
        );
        assert!(script.matches_url("https://example.com/"));
        assert!(script.matches_url("http://sub.example.com/page"));
        assert!(!script.matches_url("https://admin.example.com/panel"));
        assert!(script.matches_url("https://www.youtube.com/watch?v=123"));
        assert!(script.matches_url("https://youtube.com/watch?v=123"));
        assert!(!script.matches_url("https://www.youtube.com/feed/subscriptions"));
    }
}
