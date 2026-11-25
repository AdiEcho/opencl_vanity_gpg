use std::{collections::HashMap, str::FromStr};

use anyhow::bail;

#[derive(Debug, Clone)]
pub struct HashPattern {
    pub pattern: String,
    pub filter: String,
    pub possibliity: f64,
}

impl HashPattern {
    pub fn shorter_suffix(source: &str, count: usize) -> anyhow::Result<Self> {
        let normalized = source.trim().replace(' ', "").to_ascii_uppercase();
        // Validate the original pattern before deriving a shorter one.
        Self::from_str(&normalized)?;
        let length = normalized.chars().count();
        if count >= length {
            bail!("--save-shorter must be less than the pattern length ({length})");
        }
        let suffix: String = normalized.chars().skip(count).collect();
        let shorter = Self::from_str(&suffix)?;
        if shorter.possibliity <= 1.0 {
            bail!("--save-shorter leaves an unconstrained pattern; keep a fixed hex digit or at least two occurrences of a wildcard");
        }
        Ok(shorter)
    }

    pub fn is_match(&self, hash: &[u8]) -> bool {
        if hash.len() != 20 {
            return false;
        }

        let hash_str = hex::encode_upper(hash);

        for (i, c) in self.pattern.chars().enumerate() {
            if c.is_ascii_hexdigit() && c != hash_str.chars().nth(i).unwrap() {
                return false;
            }
        }

        let mut map = HashMap::new();
        for (pos, c) in self.pattern.chars().enumerate() {
            if ('G'..='Z').contains(&c) {
                let hash_char = hash_str.chars().nth(pos).unwrap();
                let expect = map.entry(c).or_insert(hash_char);
                if *expect != hash_char {
                    return false;
                }
            }
        }

        true
    }
}

impl FromStr for HashPattern {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let pattern = match s.trim().replace(' ', "").to_ascii_uppercase() {
            x if x.chars().count() <= 40 => "*".repeat(40 - x.chars().count()) + &x,
            _ => bail!("Invalid pattern: {}", s),
        };

        let mut parts: Vec<String> = vec![];

        // Handle fixed 0-9A-F
        let mut fixed_pos_count: usize = 0;
        for i in 0..=4 {
            let mut mask = String::new();
            let mut value = String::new();
            let mut activated = false;
            for j in 0..8 {
                let char = *pattern.chars().nth(i * 8 + j).get_or_insert(' ');
                if char.is_ascii_hexdigit() {
                    fixed_pos_count += 1;
                    mask += "F";
                    value += &String::from(char);
                    activated = true;
                } else {
                    mask += "0";
                    value += "0";
                }
            }
            if activated {
                parts.push(format!("(h[{i}] & 0x{mask}) == 0x{value}"));
            }
        }

        // Handle wildcard G-Z
        let mut wildcard_pos_all: [Vec<usize>; (b'Z' - b'G' + 1) as usize] =
            std::default::Default::default();
        for (i, wildcard) in pattern.chars().enumerate() {
            if ('G'..='Z').contains(&wildcard) {
                wildcard_pos_all[((wildcard as u8) - b'G') as usize].push(i);
            }
        }
        let mut wildcard_pos_count = 0;

        for wildcard in 'G'..='Z' {
            let wildcard_pos = &wildcard_pos_all[((wildcard as u8) - b'G') as usize];
            if wildcard_pos.len() >= 2 {
                for i in 1..wildcard_pos.len() {
                    let left_index = wildcard_pos[i - 1] / 8;
                    let right_index = wildcard_pos[i] / 8;
                    let left_digit = 7 - wildcard_pos[i - 1] % 8;
                    let right_digit = 7 - wildcard_pos[i] % 8;
                    parts.push(format!(
                        "(/* {}: h[{}][{}] == h[{}][{}] */ (h[{}] {} {}) & 0xF{}) == (h[{}] & 0xF{})",
                        wildcard,
                        left_index,
                        left_digit,
                        right_index,
                        right_digit,
                        left_index,
                        if right_digit > left_digit { "<<" } else { ">>" },
                        right_digit.abs_diff(left_digit) * 4,
                        "0".repeat(right_digit),
                        right_index,
                        "0".repeat(right_digit),
                    ));
                }
                wildcard_pos_count += wildcard_pos.len() - 1;
            }
        }

        let filter = if !parts.is_empty() {
            parts.join(" && ")
        } else {
            String::from("true")
        };

        Ok(HashPattern {
            pattern,
            filter,
            possibliity: (16f64).powi((fixed_pos_count + wildcard_pos_count) as i32),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(suffix: &str) -> Vec<u8> {
        hex::decode(format!("{:0>40}", suffix)).unwrap()
    }

    #[test]
    fn shorter_matches_include_original_and_intermediate_lengths() {
        let original: HashPattern = "XXXXXX".parse().unwrap();
        let shorter = HashPattern::shorter_suffix(" xx xx xx ", 1).unwrap();
        assert!(shorter.is_match(&fingerprint("A77777")));
        assert!(!original.is_match(&fingerprint("A77777")));
        assert!(shorter.is_match(&fingerprint("777777")));
        assert!(original.is_match(&fingerprint("777777")));
        assert!(!shorter.is_match(&fingerprint("AA7777")));
        let shorter_two = HashPattern::shorter_suffix("XXXXXX", 2).unwrap();
        for suffix in ["AA7777", "A77777", "777777"] {
            assert!(shorter_two.is_match(&fingerprint(suffix)));
        }
    }

    #[test]
    fn shorter_keeps_suffix_alignment_and_wildcard_groups() {
        let shorter = HashPattern::shorter_suffix("12XXYY**", 2).unwrap();
        assert!(shorter.is_match(&fingerprint("ABCCDD12")));
        assert!(!shorter.is_match(&fingerprint("ABCDDD12")));
        let fixed = HashPattern::shorter_suffix("123456", 1).unwrap();
        assert!(fixed.is_match(&fingerprint("A23456")));
        assert!(!fixed.is_match(&fingerprint("123450")));
        let placeholder = HashPattern::shorter_suffix("X★XXX", 1).unwrap();
        assert!(placeholder.is_match(&fingerprint("A777")));
    }

    #[test]
    fn rejects_empty_or_unconstrained_suffixes() {
        for (source, count) in [
            ("XXXXXX", 6),
            ("XXXXXX", 7),
            ("XXXXXX", 5),
            ("AB**", 2),
            ("", 1),
        ] {
            assert!(HashPattern::shorter_suffix(source, count).is_err());
        }
    }
}
