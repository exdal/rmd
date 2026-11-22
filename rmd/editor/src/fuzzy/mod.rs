pub(crate) mod maps;

const PATH_EXACT_SEGMENT: u32 = 120;
const PATH_SEGMENT_PREFIX: u32 = 100;
const PATH_SEGMENT_WORD: u32 = 80;
const PATH_BOUNDARY: u32 = 60;
const PATH_SUBSTRING: u32 = 40;
const PATH_SUBSEQUENCE: u32 = 10;

const NAME_PREFIX: u32 = 70;
const NAME_WORD: u32 = 50;
const NAME_SUBSTRING: u32 = 30;

const EXTRA_SUBSTRING: u32 = 30;

pub(crate) struct Query {
    tokens: Vec<String>,
}

impl Query {
    pub(crate) fn parse(text: &str) -> Self {
        Self {
            tokens: text.to_lowercase().split_whitespace().map(str::to_owned).collect(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

pub(crate) struct Haystack {
    path: String,
    segment_start: usize,
    name: Option<String>,
    extra: Option<String>,
}

impl Haystack {
    pub(crate) fn new(path: &str, name: Option<&str>, extra: Option<&str>) -> Self {
        let path = path.to_lowercase();
        let segment_start = path.rfind('/').map_or(0, |slash| slash + 1);

        Self {
            path,
            segment_start,
            name: name.map(str::to_lowercase),
            extra: extra.map(str::to_lowercase),
        }
    }

    pub(crate) fn label(label: &str) -> Self {
        Self::new("", Some(label), None)
    }
}

pub(crate) fn score(query: &Query, haystack: &Haystack) -> Option<u32> {
    if query.is_empty() {
        return None;
    }

    query
        .tokens
        .iter()
        .try_fold(0, |total, token| Some(total + token_score(haystack, token)?))
}

fn token_score(haystack: &Haystack, token: &str) -> Option<u32> {
    [
        path_tier(haystack, token),
        haystack.name.as_deref().and_then(|name| name_tier(name, token)),
        haystack.extra.as_deref().and_then(|extra| extra_tier(extra, token)),
    ]
    .into_iter()
    .flatten()
    .max()
}

/// Leading tokens that match name the command and the rest are its arguments
pub(crate) fn split_arguments<'a>(query: &'a Query, haystack: &Haystack) -> Option<(u32, &'a [String])> {
    let mut total = 0;
    let mut matched = 0;
    for token in &query.tokens {
        let Some(token_score) = token_score(haystack, token) else {
            break;
        };

        total += token_score;
        matched += 1;
    }

    (matched > 0 && matched < query.tokens.len()).then(|| (total, &query.tokens[matched..]))
}

fn occurrences<'a>(text: &'a str, token: &'a str) -> impl Iterator<Item = usize> + 'a {
    text.char_indices()
        .map(|(index, _)| index)
        .filter(move |&index| text[index..].starts_with(token))
}

fn path_tier(haystack: &Haystack, token: &str) -> Option<u32> {
    let path = haystack.path.as_str();
    let segment = &path[haystack.segment_start..];
    if segment == token {
        return Some(PATH_EXACT_SEGMENT);
    }

    if segment.starts_with(token) {
        return Some(PATH_SEGMENT_PREFIX);
    }

    if occurrences(segment, token).any(|index| index > 0 && segment.as_bytes()[index - 1] == b'_') {
        return Some(PATH_SEGMENT_WORD);
    }

    if occurrences(path, token).any(|index| index > 0 && matches!(path.as_bytes()[index - 1], b'/' | b'_')) {
        return Some(PATH_BOUNDARY);
    }

    if path.contains(token) {
        return Some(PATH_SUBSTRING);
    }

    let mut remaining = path.chars();
    let is_subsequence = token.chars().all(|wanted| remaining.any(|found| found == wanted));

    is_subsequence.then_some(PATH_SUBSEQUENCE)
}

fn name_tier(name: &str, token: &str) -> Option<u32> {
    if name.starts_with(token) {
        return Some(NAME_PREFIX);
    }

    let is_word_start = occurrences(name, token).any(|index| {
        name[..index]
            .chars()
            .next_back()
            .is_none_or(|previous| !previous.is_alphanumeric())
    });
    if is_word_start {
        return Some(NAME_WORD);
    }

    name.contains(token).then_some(NAME_SUBSTRING)
}

fn extra_tier(extra: &str, token: &str) -> Option<u32> {
    extra.contains(token).then_some(EXTRA_SUBSTRING)
}

#[cfg(test)]
mod tests;
