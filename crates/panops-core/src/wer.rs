/// Word Error Rate via Levenshtein on whitespace-tokenized lowercase strings.
/// Punctuation is stripped before comparison.
pub fn wer(reference: &str, hypothesis: &str) -> f32 {
    let r = tokenize(reference);
    let h = tokenize(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let dist = levenshtein(&r, &h);
    dist as f32 / r.len() as f32
}

fn tokenize(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn levenshtein(a: &[String], b: &[String]) -> usize {
    let mut dp = vec![vec![0_usize; b.len() + 1]; a.len() + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_zero() {
        assert_eq!(wer("the quick brown fox", "the quick brown fox"), 0.0);
    }

    #[test]
    fn punctuation_and_case_ignored() {
        assert_eq!(wer("The quick, brown fox.", "the QUICK brown fox"), 0.0);
    }

    #[test]
    fn one_substitution() {
        let v = wer("the quick brown fox", "the quick brown dog");
        assert!((v - 0.25).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn empty_reference() {
        assert_eq!(wer("", ""), 0.0);
        assert_eq!(wer("", "anything"), 1.0);
    }
}
