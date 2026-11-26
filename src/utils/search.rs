use ocl::{Buffer, ProQue};

const RESULT_CAPACITY: usize = 65_536;

pub fn shader_source(filter: &str, chunks: usize, future: bool, collect_all: bool) -> String {
    include_str!("../shader.cl").replace(
        "#define __INJECTS__",
        &format!(
            "#define FILTER(h) ({filter})\n#define CHUNK ({chunks})\n#define FUTURE_MODE ({})\n#define COLLECT_ALL ({})",
            u8::from(future),
            u8::from(collect_all),
        ),
    )
}

/// Number of timestamps in a batch, including offset zero and the range endpoint.
pub fn search_size(
    dimension: usize,
    iteration: usize,
    max_range: Option<u32>,
    base: u32,
    future: bool,
) -> u64 {
    let time_limit = if future { u32::MAX - base } else { base };
    (dimension as u64)
        .saturating_mul(iteration as u64)
        .min(u64::from(max_range.filter(|&range| range > 0).unwrap_or(u32::MAX)) + 1)
        .min(u64::from(time_limit) + 1)
}

pub struct OpenClSearch {
    pro_que: ProQue,
    results: Buffer<u32>,
    collect_all: bool,
}

impl OpenClSearch {
    pub fn new(pro_que: ProQue, collect_all: bool) -> anyhow::Result<Self> {
        Self::with_capacity(
            pro_que,
            collect_all,
            if collect_all { RESULT_CAPACITY } else { 1 },
        )
    }

    fn with_capacity(pro_que: ProQue, collect_all: bool, capacity: usize) -> anyhow::Result<Self> {
        let results = Buffer::builder()
            .queue(pro_que.queue().clone())
            .len(capacity + 1)
            .build()?;
        Ok(Self {
            pro_que,
            results,
            collect_all,
        })
    }

    pub fn search(&self, hashdata: &[u32], count: u64) -> anyhow::Result<Vec<u32>> {
        let input = Buffer::<u32>::builder()
            .queue(self.pro_que.queue().clone())
            .len(hashdata.len())
            .copy_host_slice(hashdata)
            .build()?;
        let mut timestamps = Vec::new();
        self.search_range(&input, 0, count, &mut timestamps)?;
        Ok(timestamps)
    }

    fn search_range(
        &self,
        input: &Buffer<u32>,
        start: u64,
        count: u64,
        timestamps: &mut Vec<u32>,
    ) -> anyhow::Result<()> {
        if count == 0 {
            return Ok(());
        }
        // A 32-bit atomic counter must not wrap, even if every candidate matches.
        if self.collect_all && count > u64::from(u32::MAX) {
            let half = count / 2;
            self.search_range(input, start, half, timestamps)?;
            return self.search_range(input, start + half, count - half, timestamps);
        }
        // Use a blocking write: partial clEnqueueFillBuffer resets are unreliable on Apple OpenCL.
        self.results.write(&[0u32][..]).enq()?;
        let capacity = (self.results.len() - 1) as u32;
        let kernel = self
            .pro_que
            .kernel_builder("vanity_sha1")
            .arg(input)
            .arg(&self.results)
            .arg(start)
            .arg(count)
            .arg(capacity)
            .build()?;
        unsafe {
            kernel.enq()?;
        }

        let mut found = [0u32];
        self.results.read(&mut found[..]).enq()?;
        anyhow::ensure!(
            u64::from(found[0]) <= count,
            "Invalid OpenCL result count {} for {count} candidates starting at {start}",
            found[0]
        );
        if found[0] > capacity {
            // Discard the incomplete batch and rerun disjoint halves so no hits are lost.
            let half = count / 2;
            self.search_range(input, start, half, timestamps)?;
            self.search_range(input, start + half, count - half, timestamps)?;
        } else if found[0] > 0 {
            let mut batch = vec![0; found[0] as usize];
            self.results.read(&mut batch).offset(1).enq()?;
            timestamps.extend(batch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{manually_prepare_sha1, HashPattern};
    use pgp::crypto::hash::HashAlgorithm;

    #[test]
    fn counts_range_endpoints_and_timestamp_limits() {
        assert_eq!(search_size(8, 8, Some(10), 100, false), 11);
        assert_eq!(search_size(8, 8, Some(0), 100, false), 64);
        assert_eq!(search_size(8, 8, None, 2, false), 3);
        assert_eq!(search_size(8, 8, None, u32::MAX - 2, true), 3);
    }

    fn gpu(filter: &str, chunks: usize, future: bool, all: bool, capacity: usize) -> OpenClSearch {
        let pro_que = ProQue::builder()
            .src(shader_source(filter, chunks, future, all))
            .dims(64)
            .build()
            .unwrap();
        OpenClSearch::with_capacity(pro_que, all, capacity).unwrap()
    }

    #[test]
    #[ignore = "requires an OpenCL device"]
    fn collects_all_matches_even_when_the_buffer_overflows() {
        let mut raw = vec![0x99, 0, 6, 4, 0, 0, 0, 0, 22];
        let original: HashPattern = "XXX".parse().unwrap();
        let shorter = HashPattern::shorter_suffix("XXX", 1).unwrap();
        for future in [false, true] {
            raw[4..8].copy_from_slice(&10_000u32.to_be_bytes());
            let data = manually_prepare_sha1(raw.clone());
            let search = gpu(&shorter.filter, data.len() / 16, future, true, 7);
            let mut expected = Vec::new();
            let mut full_matches = 0;
            for offset in 0..4096u32 {
                let timestamp = if future {
                    10_000 + offset
                } else {
                    10_000 - offset
                };
                raw[4..8].copy_from_slice(&timestamp.to_be_bytes());
                let hash = HashAlgorithm::SHA1.digest(&raw).unwrap();
                if shorter.is_match(&hash) {
                    expected.push(timestamp);
                }
                if original.is_match(&hash) {
                    full_matches += 1;
                }
            }
            assert!(expected.len() > 7 && full_matches > 0);
            let mut actual = search.search(&data, 4096).unwrap();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    #[ignore = "requires an OpenCL device"]
    fn preserves_timestamp_zero_and_default_single_result_mode() {
        let data = manually_prepare_sha1(vec![0x99, 0, 6, 4, 0, 0, 0, 0, 22]);
        let search = gpu("true", data.len() / 16, true, true, 2);
        let mut actual = search.search(&data, 8).unwrap();
        actual.sort_unstable();
        assert_eq!(actual, (0..8).collect::<Vec<_>>());
        let search = gpu("true", data.len() / 16, true, false, 1);
        assert_eq!(search.search(&data, 1).unwrap(), vec![0]);
        let actual = search.search(&data, 128).unwrap();
        assert_eq!(actual.len(), 1);
        assert!(actual[0] < 128);
    }

    #[test]
    #[ignore = "requires an OpenCL device"]
    fn matches_full_cpu_sha1_across_blocks_and_timestamp_boundaries() {
        // Include one-block inputs, padding boundaries, and RSA-sized inputs.
        for length in [51, 56, 119, 120, 196, 532] {
            for future in [false, true] {
                let base = if future { u32::MAX - 256 } else { 256 };
                let mut raw: Vec<u8> = (0..length)
                    .map(|i| (i as u8).wrapping_mul(37).wrapping_add(11))
                    .collect();
                raw[4..8].copy_from_slice(&base.to_be_bytes());
                let data = manually_prepare_sha1(raw.clone());
                // Match every bit of all five words, including offsets that
                // require multiple iterations of the 64-work-item test kernel.
                let mut expected = Vec::new();
                let filters: Vec<String> = [0, 1, 63, 64, 255, 256]
                    .into_iter()
                    .map(|offset| {
                        let timestamp = if future { base + offset } else { base - offset };
                        expected.push(timestamp);
                        raw[4..8].copy_from_slice(&timestamp.to_be_bytes());
                        let hash = HashAlgorithm::SHA1.digest(&raw).unwrap();
                        let words: Vec<String> = hash
                            .chunks_exact(4)
                            .enumerate()
                            .map(|(i, bytes)| {
                                let word = u32::from_be_bytes(bytes.try_into().unwrap());
                                format!("h[{i}] == 0x{word:08X}u")
                            })
                            .collect();
                        format!("({})", words.join(" && "))
                    })
                    .collect();
                let search = gpu(&filters.join(" || "), data.len() / 16, future, true, 16);
                let mut actual = search.search(&data, 257).unwrap();
                actual.sort_unstable();
                expected.sort_unstable();
                assert_eq!(actual, expected, "length={length}, future={future}");
            }
        }
    }
}
