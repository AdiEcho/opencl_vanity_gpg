use anyhow::bail;
use log::*;
use ocl::{Device, Platform, ProQue};
use pgp::types::PublicKeyTrait;
use rand::thread_rng;
use std::{
    fs,
    path::Path,
    sync::{mpsc::*, LazyLock},
    thread,
    time::Instant,
};
use utils::ARGS;
use utils::*;

mod utils;

fn main() -> anyhow::Result<()> {
    let bars = init_logger();

    debug!("{:#?}", LazyLock::force(&ARGS));

    let (pattern, shorter_pattern) = ARGS.patterns()?;
    let device_list = utils::DeviceList::new()?;

    if ARGS.list_device {
        info!("Available OpenCL devices: \n");
        for (i, device) in device_list.iter().enumerate() {
            println!("Device #{i} - {device:?}");
        }
        return Ok(());
    }

    let (filter, estimate) = match &ARGS.filter {
        Some(filter) => (filter.clone(), None),
        None => match &pattern {
            Some(p) => (
                shorter_pattern.as_ref().unwrap_or(p).filter.clone(),
                Some(p.possibliity),
            ),
            None => bail!("No filter or pattern given"),
        },
    };
    // A custom filter takes precedence over --pattern, including CPU verification.
    let pattern = pattern.filter(|_| ARGS.filter.is_none());
    if let Some(shorter) = &shorter_pattern {
        info!(
            "Also saving shorter suffix matches: {} (--oneshot waits for the original pattern)",
            shorter.pattern.trim_start_matches('*')
        );
    }
    debug!("Filter: {filter}");

    let device = match ARGS.device {
        Some(i) => device_list[i].device,
        None => Device::first(Platform::default())?,
    };

    info!("Using device: {}", device.name()?);

    let dimension = match ARGS.thread {
        Some(v) => v,
        None => match device.info(ocl::core::DeviceInfo::MaxWorkItemSizes)? {
            ocl::core::DeviceInfoResult::MaxWorkItemSizes(wgs) => {
                let dimension = usize::max(wgs[0] * wgs[1], 1048576);
                info!("Auto set thread: {dimension}");
                dimension
            }
            _ => unreachable!(),
        },
    };

    let iteration = ARGS.iteration;
    if dimension == 0 || iteration == 0 {
        bail!("--thread and --iteration must be greater than zero");
    }

    // Determine the starting timestamp
    let start_timestamp = ARGS
        .start_timestamp
        .unwrap_or_else(|| chrono::Utc::now().timestamp());

    // Use the user-specified time range instead of dimension * iteration
    info!(
        "Starting search from {} and going {} in time{}",
        chrono::DateTime::from_timestamp(start_timestamp, 0)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        if ARGS.future_timestamp {
            "forward"
        } else {
            "backward"
        },
        if ARGS.max_time_range.is_some() {
            format!(" (up to {} seconds)", ARGS.max_time_range.unwrap())
        } else {
            "".to_string()
        },
    );

    if ARGS.output.is_none() {
        if ARGS.no_secret_key_logging {
            warn!("No output dir given and you disabled secret key logging. You have no chance to save generated vanity keys.");
        } else {
            warn!("No output dir given. Generated vanity keys will not be saved.");
        }
    }

    if let Some(ref output_dir) = ARGS.output {
        warn!("Output directory does not exist, creating it: {output_dir}");
        fs::create_dir_all(output_dir)?;
    }

    let mut rng = thread_rng();

    match ARGS.cipher_suite {
        CipherSuite::RSA2048 | CipherSuite::RSA3072 | CipherSuite::RSA4096 => {
            warn!("Generating RSA vanity keys is not recommended. Too slow!")
        }
        _ => (),
    };

    let mut vanity_key = VanitySecretKey::new(ARGS.cipher_suite, ARGS.user_id.clone(), &mut rng);

    // Set the initial timestamp for the key
    // Ensure we use the full timestamp value, not truncated
    let initial_timestamp = if start_timestamp > u32::MAX as i64 {
        // If timestamp is too large for u32, we need to handle this carefully
        // For now, use the current approach but ensure we're aware of the limitation
        warn!(
            "Timestamp {} is too large for u32, may cause unexpected behavior",
            start_timestamp
        );
        start_timestamp as u32
    } else {
        start_timestamp as u32
    };

    info!(
        "Using base timestamp: {} ({})",
        initial_timestamp,
        chrono::DateTime::from_timestamp(initial_timestamp as i64, 0)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    );

    vanity_key.edit_timestamp(initial_timestamp, &mut rng);

    let mut hashdata = manually_prepare_sha1(vanity_key.hashdata());

    let (tx_hashdata, rx_hashdata) = channel::<Vec<u32>>();
    let (tx_result, rx_result) = channel::<anyhow::Result<Vec<u32>>>();

    let mut hashed = 0;
    let mut start = Instant::now();
    let run_start = start;

    let pro_que = ProQue::builder()
        .src(shader_source(
            &filter,
            hashdata.len() / 16,
            ARGS.future_timestamp,
            shorter_pattern.is_some(),
        ))
        .device(device)
        .dims(dimension)
        .build()?;

    let search = OpenClSearch::new(pro_que, shorter_pattern.is_some())?;
    let bench_size = search_size(
        dimension,
        iteration,
        ARGS.max_time_range,
        initial_timestamp,
        ARGS.future_timestamp,
    );
    thread::spawn(move || opencl_thread(search, bench_size, rx_hashdata, tx_result));
    let bar = bars.add(init_progress_bar(estimate));

    loop {
        debug!("Send key to OpenCL thread");
        tx_hashdata.send(hashdata)?;
        let mut vanity_key_next =
            VanitySecretKey::new(ARGS.cipher_suite, ARGS.user_id.clone(), &mut rng);

        // Set the same initial timestamp for consistency
        vanity_key_next.edit_timestamp(initial_timestamp, &mut rng);
        let hashdata_next = manually_prepare_sha1(vanity_key_next.hashdata());

        debug!("Receive result from OpenCL thread");
        let vanity_timestamps = rx_result.recv()??;
        hashed += bench_size;

        let elapsed = start.elapsed().as_secs_f64();
        bar.inc(bench_size);

        let mut matched_original = false;
        for vanity_timestamp in vanity_timestamps {
            vanity_key.edit_timestamp(vanity_timestamp, &mut rng);

            if let Some(accepted) = shorter_pattern.as_ref().or(pattern.as_ref()) {
                if !vanity_key.check_pattern(accepted) {
                    bail!("OpenCL result failed fingerprint verification at timestamp {vanity_timestamp}");
                }
            }
            let original_match = pattern.as_ref().is_none_or(|p| vanity_key.check_pattern(p));
            matched_original |= original_match;
            if shorter_pattern.is_some() {
                info!(
                    "Matched {} pattern",
                    if original_match {
                        "original"
                    } else {
                        "shorter suffix"
                    }
                );
            }
            vanity_key.log_state();

            if let Some(ref output_dir) = ARGS.output {
                fs::write(
                    Path::new(output_dir).join(format!(
                        "{}-sec.asc",
                        hex::encode_upper(vanity_key.secret_key.fingerprint().as_bytes())
                    )),
                    vanity_key.to_armored_string()?,
                )?;
            }
        }

        if matched_original {
            match estimate {
                Some(estimate) => info!(
                    "Hashed: {} ({:.02}x) Time: {:.02}s Speed: {} hash/s",
                    format_number(hashed as f64),
                    (hashed as f64) / estimate,
                    elapsed,
                    format_number((hashed as f64) / elapsed),
                ),
                None => info!(
                    "Hashed: {} Time: {:.02}s Speed: {} hash/s",
                    format_number(hashed as f64),
                    elapsed,
                    format_number((hashed as f64) / elapsed),
                ),
            }

            // Save every match in this batch before honoring --oneshot.
            if ARGS.oneshot {
                bar.finish();
                bars.clear()?;
                break;
            }

            hashed = 0;
            bar.reset();
            start = Instant::now();
        }

        if let Some(timeout) = ARGS.timeout {
            if run_start.elapsed().as_secs_f64() > timeout {
                info!("Timeout!");
                break;
            }
        }

        vanity_key = vanity_key_next;
        hashdata = hashdata_next;
    }

    Ok(())
}

fn opencl_thread(
    search: OpenClSearch,
    count: u64,
    rx_hashdata: Receiver<Vec<u32>>,
    tx_result: Sender<anyhow::Result<Vec<u32>>>,
) {
    debug!("OpenCL thread ready");
    while let Ok(hashdata) = rx_hashdata.recv() {
        if tx_result.send(search.search(&hashdata, count)).is_err() {
            break;
        }
    }
    debug!("OpenCL thread quit");
}
