use crate::message_normalization::content_to_text;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Add, Div, Mul, Sub};

pub const DEFAULT_TERM_SATURATION: f64 = 1.5;
pub const DEFAULT_LENGTH_NORMALIZATION: f64 = 0.75;
const MINIMUM_PREFIX_LENGTH: usize = 4;

struct ScoringContext<'context> {
    average_length: f64,
    idf: &'context BTreeMap<String, f64>,
    query_terms: &'context [String],
    term_saturation: f64,
    length_normalization: f64,
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn tokenized_documents(chat_messages: &[Value]) -> Vec<Vec<String>> {
    chat_messages
        .iter()
        .map(|message| {
            let content = message
                .get("content")
                .map(content_to_text)
                .unwrap_or_default();
            tokenize(&content)
        })
        .collect()
}

fn average_document_length(documents: &[Vec<String>]) -> f64 {
    let summed_length = documents
        .iter()
        .map(Vec::len)
        .fold(0_usize, usize::saturating_add);
    let total_as_f64 = u32::try_from(summed_length).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let documents_as_f64 =
        u32::try_from(documents.len()).map_or_else(|_| f64::from(u32::MAX), f64::from);
    Div::div(total_as_f64, documents_as_f64)
}

fn document_frequencies(documents: &[Vec<String>]) -> BTreeMap<&str, usize> {
    let mut frequencies: BTreeMap<&str, usize> = BTreeMap::new();
    for tokens in documents {
        let unique: BTreeSet<&str> = tokens.iter().map(String::as_str).collect();
        for term in unique {
            frequencies
                .entry(term)
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
        }
    }
    frequencies
}

fn inverse_document_frequencies(
    query_terms: &[String],
    frequencies: &BTreeMap<&str, usize>,
    document_count: usize,
) -> BTreeMap<String, f64> {
    let documents_as_f64 =
        u32::try_from(document_count).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let mut idf: BTreeMap<String, f64> = BTreeMap::new();
    let unique: BTreeSet<&str> = query_terms.iter().map(String::as_str).collect();
    for term in unique {
        let occurrences = frequencies.get(term).copied().unwrap_or(0);
        let count_as_f64 =
            u32::try_from(occurrences).map_or_else(|_| f64::from(u32::MAX), f64::from);
        let difference = Sub::sub(documents_as_f64, count_as_f64);
        let numerator = Add::add(difference, 0.5_f64);
        let denominator = Add::add(count_as_f64, 0.5_f64);
        let ratio = Div::div(numerator, denominator);
        idf.insert(term.to_owned(), Add::add(ratio, 1.0_f64).ln());
    }
    idf
}

fn term_frequency(term: &str, token_counts: &BTreeMap<&str, usize>) -> usize {
    let exact = token_counts.get(term).copied().unwrap_or(0);
    if exact > 0 {
        return exact;
    }
    if term.len() < MINIMUM_PREFIX_LENGTH {
        return 0;
    }
    token_counts
        .iter()
        .filter(|(token, _)| token.starts_with(term))
        .map(|(_, count)| *count)
        .fold(0_usize, usize::saturating_add)
}

fn saturation_factor(document_length: f64, context: &ScoringContext<'_>) -> f64 {
    let scaled_length = Div::div(
        Mul::mul(context.length_normalization, document_length),
        context.average_length,
    );
    let length_complement = Sub::sub(1.0_f64, context.length_normalization);
    Mul::mul(
        context.term_saturation,
        Add::add(length_complement, scaled_length),
    )
}

fn term_weight(idf: f64, occurrences: usize, saturation_numerator: f64, saturation: f64) -> f64 {
    let count_as_f64 = u32::try_from(occurrences).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let numerator = Mul::mul(count_as_f64, saturation_numerator);
    let denominator = Add::add(count_as_f64, saturation);
    Div::div(Mul::mul(idf, numerator), denominator)
}

fn document_score(tokens: &[String], context: &ScoringContext<'_>) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }
    let mut token_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for token in tokens {
        token_counts
            .entry(token.as_str())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
    }
    let document_length =
        u32::try_from(tokens.len()).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let saturation = saturation_factor(document_length, context);
    let saturation_numerator = Add::add(context.term_saturation, 1.0_f64);
    let mut score = 0.0_f64;
    for term in context.query_terms {
        let Some(idf_value) = context.idf.get(term.as_str()) else {
            continue;
        };
        let occurrences = term_frequency(term, &token_counts);
        if occurrences == 0 {
            continue;
        }
        score = Add::add(
            score,
            term_weight(*idf_value, occurrences, saturation_numerator, saturation),
        );
    }
    score
}

pub fn bm25_score_messages(
    query: &str,
    messages: &[Value],
    term_saturation: f64,
    length_normalization: f64,
) -> Vec<f64> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return vec![0.0; messages.len()];
    }
    let documents = tokenized_documents(messages);
    if documents.is_empty() {
        return Vec::new();
    }
    let frequencies = document_frequencies(&documents);
    let idf = inverse_document_frequencies(&query_terms, &frequencies, documents.len());
    let context = ScoringContext {
        average_length: average_document_length(&documents),
        idf: &idf,
        query_terms: query_terms.as_slice(),
        term_saturation,
        length_normalization,
    };
    documents
        .iter()
        .map(|tokens| document_score(tokens, &context))
        .collect()
}

mod tests;
