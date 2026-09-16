use std::sync::OnceLock;

use regex::Regex;

/// Patterns to include/exclude and quantization tokens to keep.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub quants: Vec<String>,
}

impl Filter {
    pub fn matches(&self, path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        if self
            .exclude
            .iter()
            .any(|pattern| !pattern.is_empty() && lower.contains(&pattern.to_ascii_lowercase()))
        {
            return false;
        }

        if !self.include.is_empty()
            && !self
                .include
                .iter()
                .any(|pattern| lower.contains(&pattern.to_ascii_lowercase()))
        {
            return false;
        }

        if !self.quants.is_empty() {
            return match gguf_quant(path) {
                Some(quant) => self
                    .quants
                    .iter()
                    .any(|wanted| wanted.eq_ignore_ascii_case(&quant)),
                None => false,
            };
        }

        true
    }
}

/// Split `org/repo:q4_k_m,q5_k_m` into the repo and requested quants.
pub fn parse_source(source: &str) -> (String, Vec<String>) {
    if let Some(index) = source.rfind(':') {
        let repo = &source[..index];
        let rest = &source[index + 1..];
        if !repo.is_empty() && !repo.contains(':') && !rest.is_empty() && !rest.contains('/') {
            let quants = rest
                .split(',')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect();
            return (repo.to_string(), quants);
        }
    }
    (source.to_string(), Vec::new())
}

fn quant_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[._\-\s])(iq[1-4]_[a-z0-9]+|q[2-8]_k_[sml]|q[2-8]_k|q[2-8]_[01]|q[2-8]|[bf]?f(?:16|32)|bf16)(?:[._\-\s]|$)",
        )
        .expect("valid quant regex")
    })
}

/// Extract a GGUF quantization token from a filename, e.g. `Q4_K_M`.
pub fn gguf_quant(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let captures = quant_regex().captures(name)?;
    let token = captures.get(1)?.as_str().to_ascii_uppercase();
    Some(normalize_quant(&token))
}

fn normalize_quant(token: &str) -> String {
    match token {
        "FP16" => "F16".to_string(),
        "FP32" => "F32".to_string(),
        other => other.to_string(),
    }
}

/// Relative quality rating, 1 (worst) to 5 (best).
pub fn quality_stars(quant: &str) -> u8 {
    match quant.to_ascii_uppercase().as_str() {
        "IQ1_S" | "IQ1_M" => 1,
        "Q2_K" | "IQ2_XXS" | "IQ2_XS" | "IQ2_S" | "IQ2_M" | "Q3_K_S" | "IQ3_XXS" => 2,
        "Q3_K_M" | "Q3_K_L" | "IQ3_XS" | "IQ3_S" | "IQ3_M" | "Q4_0" | "Q4_1" => 3,
        "Q4_K_S" | "Q4_K_M" | "IQ4_XS" | "IQ4_NL" | "Q5_0" | "Q5_1" => 4,
        "Q5_K_S" | "Q5_K_M" | "Q6_K" | "Q8_0" | "F16" | "F32" | "BF16" => 5,
        _ => 3,
    }
}

pub fn stars(quant: &str) -> String {
    let filled = quality_stars(quant) as usize;
    let mut out = String::new();
    for i in 0..5 {
        out.push(if i < filled { '★' } else { '☆' });
    }
    out
}

/// Rough RAM needed to run a model of the given file size, in bytes.
pub fn ram_estimate(file_size: u64) -> u64 {
    file_size + file_size / 12 + 300 * 1024 * 1024
}

pub fn is_recommended(quant: &str) -> bool {
    quant.eq_ignore_ascii_case("Q4_K_M")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_detection() {
        assert_eq!(
            gguf_quant("mistral-7b.Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(gguf_quant("model-q5_k_s.gguf").as_deref(), Some("Q5_K_S"));
        assert_eq!(gguf_quant("llama.Q8_0.gguf").as_deref(), Some("Q8_0"));
        assert_eq!(gguf_quant("model.iq2_xxs.gguf").as_deref(), Some("IQ2_XXS"));
        assert_eq!(gguf_quant("model.f16.gguf").as_deref(), Some("F16"));
        assert_eq!(gguf_quant("config.json"), None);
    }

    #[test]
    fn source_parsing() {
        assert_eq!(
            parse_source("TheBloke/Mistral-7B-GGUF:q4_k_m,q5_k_m"),
            (
                "TheBloke/Mistral-7B-GGUF".to_string(),
                vec!["q4_k_m".to_string(), "q5_k_m".to_string()]
            )
        );
        assert_eq!(
            parse_source("owner/name"),
            ("owner/name".to_string(), Vec::new())
        );
    }

    #[test]
    fn filtering() {
        let filter = Filter {
            include: vec![],
            exclude: vec!["README".to_string()],
            quants: vec!["q4_k_m".to_string()],
        };
        assert!(filter.matches("model.Q4_K_M.gguf"));
        assert!(!filter.matches("model.Q5_K_M.gguf"));
        assert!(!filter.matches("README.md"));
    }
}
