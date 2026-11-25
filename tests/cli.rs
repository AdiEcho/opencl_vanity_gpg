use pgp::{types::PublicKeyTrait, Deserializable, SignedSecretKey};
use std::{fs, path::PathBuf, process::Command, time::SystemTime};

struct OutputDirectory(PathBuf);

impl OutputDirectory {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!("vanity-gpg-test-{}-{unique}", std::process::id())))
    }
}

impl Drop for OutputDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str], output: &OutputDirectory) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_opencl_vanity_gpg"))
        .args(args)
        .args([
            "--thread",
            "1024",
            "--iteration",
            "64",
            "--no-secret-key-logging",
            "--no-progress",
            "--output",
        ])
        .arg(&output.0)
        .env("RUST_LOG", "info")
        .output()
        .unwrap();
    let log = String::from_utf8(result.stderr).unwrap();
    assert!(result.status.success(), "{log}");
    log
}

fn saved_fingerprints(output: &OutputDirectory, subkey: bool) -> Vec<String> {
    fs::read_dir(&output.0)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            let (key, _) = SignedSecretKey::from_armor_single(&bytes[..]).unwrap();
            key.verify().unwrap();
            assert_eq!(
                path.file_name().unwrap().to_str().unwrap(),
                format!(
                    "{}-sec.asc",
                    hex::encode_upper(key.fingerprint().as_bytes())
                )
            );
            hex::encode_upper(
                if subkey {
                    key.secret_subkeys[0].fingerprint()
                } else {
                    key.fingerprint()
                }
                .as_bytes(),
            )
        })
        .collect()
}

fn repeated_suffix(fingerprint: &str, length: usize) -> bool {
    let bytes = fingerprint.as_bytes();
    bytes[bytes.len() - length..]
        .iter()
        .all(|&byte| byte == bytes[bytes.len() - 1])
}

#[test]
#[ignore = "requires an OpenCL device"]
fn saves_valid_shorter_and_original_keys_before_oneshot_exits() {
    for (suite, subkey) in [("ed25519", false), ("cv25519", true)] {
        let output = OutputDirectory::new();
        let mut args = vec![
            "--cipher-suite",
            suite,
            "--pattern",
            "XXXX",
            "--save-shorter",
            "1",
            "--oneshot",
            "--timeout",
            "30",
        ];
        if subkey {
            args.push("--future-timestamp");
        }
        let log = run(&args, &output);
        assert!(!log.contains("Timeout!"), "{log}");
        let fingerprints = saved_fingerprints(&output, subkey);
        assert!(fingerprints.iter().all(|fp| repeated_suffix(fp, 3)));
        assert!(
            fingerprints.iter().any(|fp| repeated_suffix(fp, 4)),
            "{log}"
        );
        assert!(
            fingerprints.iter().any(|fp| !repeated_suffix(fp, 4)),
            "{log}"
        );
        assert_eq!(fingerprints.len(), log.matches("Get a vanity key!").count());
    }
}

#[test]
#[ignore = "requires an OpenCL device"]
fn zero_shortening_preserves_single_result_and_filter_precedence() {
    let output = OutputDirectory::new();
    run(
        &[
            "--pattern",
            "XXX",
            "--save-shorter",
            "0",
            "--oneshot",
            "--timeout",
            "30",
        ],
        &output,
    );
    let fingerprints = saved_fingerprints(&output, false);
    assert_eq!(fingerprints.len(), 1);
    assert!(repeated_suffix(&fingerprints[0], 3));

    let output = OutputDirectory::new();
    run(
        &[
            "--pattern",
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
            "--filter",
            "true",
            "--oneshot",
        ],
        &output,
    );
    assert_eq!(saved_fingerprints(&output, false).len(), 1);
}

#[test]
#[ignore = "requires an OpenCL device"]
fn saves_shorter_keys_without_original_and_exits_on_timeout() {
    let output = OutputDirectory::new();
    let log = run(
        &[
            "--pattern",
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
            "--save-shorter",
            "38",
            "--oneshot",
            "--timeout",
            "0",
        ],
        &output,
    );
    let fingerprints = saved_fingerprints(&output, false);
    assert!(!fingerprints.is_empty());
    assert!(fingerprints.iter().all(|fp| fp.ends_with("FF")));
    assert!(log.contains("Timeout!"));
    assert!(!log.contains("Matched original pattern"));
    assert_eq!(
        fingerprints.len(),
        log.matches("Matched shorter suffix pattern").count()
    );
}
