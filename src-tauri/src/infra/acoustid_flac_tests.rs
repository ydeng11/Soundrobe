use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    audio: PathBuf,
    fpcalc: PathBuf,
    ffmpeg: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("verified-pcm-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            audio: root.join("source.flac"),
            fpcalc: root.join("fpcalc"),
            ffmpeg: root.join("ffmpeg"),
            root,
        };
        let mut header = vec![0; 42];
        header[..8].copy_from_slice(b"fLaC\x80\0\0\x22");
        let format = (44100_u64 << 44) | (1 << 41) | (15 << 36) | 1;
        header[18..26].copy_from_slice(&format.to_be_bytes());
        header[26..].copy_from_slice(&[
            0x09, 0x8f, 0x6b, 0xcd, 0x46, 0x21, 0xd3, 0x73, 0xca, 0xde, 0x4e, 0x83, 0x26, 0x27,
            0xb4, 0xf6,
        ]); // MD5 of exactly the four decoded bytes "test".
        fs::write(&fixture.audio, header).unwrap();
        fixture.script(&fixture.ffmpeg, "[ \"$*\" = '-nostdin -v error -noauto_conversion_filters -threads 1 -f flac -i pipe:0 -map 0:a:0 -c:a pcm_s16le -ar 44100 -ac 2 -f s16le pipe:1' ] || exit 9\ncat >/dev/null\nprintf test\n");
        fixture.script(&fixture.fpcalc, concat!(
            "if [ \"$5\" = -format ]; then\n",
            "[ \"$*\" = '-json -raw -length 0 -format s16le -rate 44100 -channels 2 -' ] || exit 9\n",
            "[ \"$(cat)\" = test ] || exit 8\n",
            "printf '%s' '{\"fingerprint\":[1,2,3]}'\n",
            "else printf '%s' '{\"fingerprint\":[999]}'; exit 3; fi\n",
        ));
        fixture
    }

    fn script(&self, path: &Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    async fn fingerprint(&self) -> Result<Vec<u32>, AcoustidError> {
        self.run(
            &AtomicBool::new(false),
            // These classify output, not startup latency of fresh executables.
            Duration::from_secs(10),
            FingerprintLimits::default(),
        )
        .await
    }

    async fn run(
        &self,
        cancelled: &AtomicBool,
        timeout: Duration,
        limits: FingerprintLimits,
    ) -> Result<Vec<u32>, AcoustidError> {
        let output = fingerprint_output(
            FingerprintTools {
                fpcalc: &self.fpcalc,
                ffmpeg: Some(&self.ffmpeg),
            },
            &self.audio,
            cancelled,
            timeout,
            limits,
            true,
        )
        .await?;
        parse_raw_output(&output)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn accept_any_pcm(fixture: &Fixture, output: &str, status: i32) {
    fixture.script(&fixture.fpcalc, &format!(
        "if [ \"$5\" = -format ]; then cat >/dev/null; printf '%s' '{output}'; exit {status}; else printf '%s' '{{\"fingerprint\":[999]}}'; exit 3; fi\n"
    ));
}

#[tokio::test]
async fn verified_pcm_refuses_signal_termination_and_replacement_during_the_direct_attempt() {
    for direct in [
        "kill -TERM $$",
        "cp -p \"$5\" \"$5.new\"; mv \"$5.new\" \"$5\"; exit 3",
    ] {
        let fixture = Fixture::new();
        fixture.script(&fixture.fpcalc, &format!(
            "if [ \"$5\" = -format ]; then cat >/dev/null; printf '%s' '{{\"fingerprint\":[1,2,3]}}'; else {direct}; fi\n"
        ));
        assert_eq!(
            fixture.fingerprint().await,
            Err(AcoustidError::ProcessFailed),
            "{direct}"
        );
    }
}

#[tokio::test]
async fn verified_pcm_rejects_in_place_audio_replacement_with_restored_modification_time() {
    use md5::Digest;
    let fixture = Fixture::new();
    let replacement = fixture.root.join("source.flac.replacement");
    let mut header = fs::read(&fixture.audio).unwrap();
    header[26..42].copy_from_slice(&md5::Md5::digest(b"TEST"));
    fs::write(&replacement, header).unwrap();
    let original_modified = fs::metadata(&fixture.audio).unwrap().modified().unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(original_modified))
        .unwrap();
    fixture.script(&fixture.ffmpeg, "cat >/dev/null\nprintf TEST\n");
    fixture.script(
        &fixture.fpcalc,
        concat!(
        "if [ \"$5\" = -format ]; then cat >/dev/null; printf '%s' '{\"fingerprint\":[1,2,3]}'; ",
        "else cat \"$5.replacement\" > \"$5\"; touch -r \"$5.replacement\" \"$5\"; exit 3; fi\n",
    ),
    );
    let before = fs::metadata(&fixture.audio).unwrap();
    let result = fixture.fingerprint().await;
    let after = fs::metadata(&fixture.audio).unwrap();
    use std::os::unix::fs::MetadataExt;
    assert_eq!(
        (
            before.dev(),
            before.ino(),
            before.len(),
            before.modified().unwrap()
        ),
        (
            after.dev(),
            after.ino(),
            after.len(),
            after.modified().unwrap()
        )
    );
    assert_eq!(result, Err(AcoustidError::ProcessFailed));
}

#[tokio::test]
async fn verified_pcm_execution_budget_starts_after_waiting_for_other_jobs() {
    let fixture = Fixture::new();
    let semaphore = FP_SEMAPHORE.get_or_init(|| Arc::new(Semaphore::new(2)));
    let permits = semaphore.acquire_many(2).await.unwrap();
    fixture.script(
        &fixture.fpcalc,
        "printf '%s' '{\"duration\":10,\"fingerprint\":\"abc\"}'\n",
    );
    let cancelled = AtomicBool::new(false);
    let mut result = Box::pin(fingerprint_file_with(
        &fixture.fpcalc,
        &fixture.audio,
        &cancelled,
        Duration::from_secs(1),
        1024,
        1024,
    ));
    tokio::select! {
        result = &mut result => panic!("queued work spent its execution budget: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(1100)) => {},
    }
    drop(permits);
    assert_eq!(result.await.unwrap().value, "abc");
}

#[tokio::test]
async fn verified_pcm_does_not_wait_for_a_fifo_writer() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let original = flac::source_snapshot(&fixture.audio).unwrap();
    fs::remove_file(&fixture.audio).unwrap();
    let name = std::ffi::CString::new(fixture.audio.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let released = Arc::new(AtomicBool::new(false));
    let writer_flag = Arc::clone(&released);
    let path = fixture.audio.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        writer_flag.store(true, Ordering::Release);
        // Release a blocking old open, without hanging the corrected case
        // where no reader was opened. This bounds the regression itself.
        let _ = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path);
    });
    let cancelled = AtomicBool::new(false);
    let job = FingerprintJob {
        cancelled: &cancelled,
        deadline: tokio::time::Instant::now() + Duration::from_secs(3),
        limits: FingerprintLimits::default(),
    };
    let result = flac::fingerprint(
        FingerprintTools {
            fpcalc: &fixture.fpcalc,
            ffmpeg: Some(&fixture.ffmpeg),
        },
        &fixture.audio,
        &job,
        Some(&original),
    )
    .await;
    let waited_for_writer = released.load(Ordering::Acquire);
    writer.join().unwrap();
    assert_eq!(result, Err(AcoustidError::ProcessFailed));
    assert!(
        !waited_for_writer,
        "a non-regular source must not block before the file-type check"
    );
}

#[tokio::test]
async fn verified_pcm_never_returns_success_after_cancellation_becomes_ready() {
    let cancelled = AtomicBool::new(false);
    let job = FingerprintJob {
        cancelled: &cancelled,
        deadline: tokio::time::Instant::now() + Duration::from_secs(1),
        limits: FingerprintLimits::default(),
    };
    assert_eq!(
        job.wait(async {
            // A final verification poll may finish at the same time cancellation
            // arrives. Completed work must not win that race and authorize a tag.
            cancelled.store(true, Ordering::Release);
            Ok(())
        })
        .await,
        Err(AcoustidError::Cancelled)
    );
}

#[tokio::test]
async fn verified_pcm_recovers_complete_audio_without_accepting_failed_direct_frames() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.audio).unwrap();
    assert_eq!(fixture.fingerprint().await.unwrap(), [1, 2, 3]);
    assert_eq!(fs::read(&fixture.audio).unwrap(), before);
}

#[tokio::test]
async fn verified_pcm_requires_exact_checksum_and_complete_sample_count() {
    for decoded in ["TEST", "tes", "tests", ""] {
        let fixture = Fixture::new();
        // The fingerprint tool happily accepts these bytes. Integrity, not
        // the existence of plausible fingerprint JSON, must reject them.
        accept_any_pcm(&fixture, r#"{"fingerprint":[1,2,3]}"#, 0);
        fixture.script(
            &fixture.ffmpeg,
            &format!("cat >/dev/null\nprintf '%s' '{decoded}'\n"),
        );
        assert_eq!(
            fixture.fingerprint().await,
            Err(AcoustidError::ProcessFailed),
            "{decoded}"
        );
    }
}

fn native_24_bit_fixture(channels: u64) -> Fixture {
    use md5::Digest;
    let fixture = Fixture::new();
    let pcm = if channels == 1 { "abc" } else { "abcdef" };
    let mut header = fs::read(&fixture.audio).unwrap();
    let format = (48000_u64 << 44) | ((channels - 1) << 41) | (23 << 36) | 1;
    header[18..26].copy_from_slice(&format.to_be_bytes());
    header[26..42].copy_from_slice(&md5::Md5::digest(pcm));
    fs::write(&fixture.audio, header).unwrap();
    fixture.script(&fixture.ffmpeg, &format!(
        "[ \"$*\" = '-nostdin -v error -noauto_conversion_filters -threads 1 -f flac -i pipe:0 -map 0:a:0 -c:a pcm_s24le -ar 48000 -ac {channels} -f s24le pipe:1' ] || exit 9\ncat >/dev/null\nprintf '{pcm}'\n"
    ));
    fixture.script(&fixture.fpcalc, &format!(
        "if [ \"$5\" = -format ]; then\n[ \"$*\" = '-json -raw -length 0 -format s24le -rate 48000 -channels {channels} -' ] || exit 9\n[ \"$(cat)\" = '{pcm}' ] || exit 8\nprintf '%s' '{{\"fingerprint\":[1,2,3]}}'\nelse exit 3; fi\n"
    ));
    fixture
}

#[tokio::test]
async fn verified_pcm_preserves_native_24_bit_mono_and_stereo_precision() {
    for channels in [1, 2] {
        let fixture = native_24_bit_fixture(channels);
        let before = fs::read(&fixture.audio).unwrap();
        assert_eq!(fixture.fingerprint().await.unwrap(), [1, 2, 3]);
        assert_eq!(fs::read(&fixture.audio).unwrap(), before);
    }
}

#[tokio::test]
async fn verified_pcm_24_bit_requires_checksum_count_and_byte_cap() {
    for decoded in ["abcdeF", "abcde", "abcdefg", ""] {
        let fixture = native_24_bit_fixture(2);
        accept_any_pcm(&fixture, r#"{"fingerprint":[1,2,3]}"#, 0);
        fixture.script(
            &fixture.ffmpeg,
            &format!("cat >/dev/null\nprintf '{decoded}'\n"),
        );
        assert_eq!(
            fixture.fingerprint().await,
            Err(AcoustidError::ProcessFailed)
        );
    }
    let fixture = native_24_bit_fixture(2);
    let mut header = fs::read(&fixture.audio).unwrap();
    let format = (48000_u64 << 44) | (1 << 41) | (23 << 36) | 44_739_243;
    header[18..26].copy_from_slice(&format.to_be_bytes());
    fs::write(&fixture.audio, header).unwrap();
    let marker = fixture.root.join("decoder-called");
    fixture.script(&fixture.ffmpeg, &format!("touch '{}'\n", marker.display()));
    assert_eq!(
        fixture.fingerprint().await,
        Err(AcoustidError::ProcessFailed)
    );
    assert!(
        !marker.exists(),
        "24-bit sample width must count toward the same byte cap"
    );
}

#[tokio::test]
async fn verified_pcm_refuses_unverifiable_or_unsupported_streaminfo_before_decoding() {
    for variant in 0..10 {
        let fixture = Fixture::new();
        let mut bytes = fs::read(&fixture.audio).unwrap();
        match variant {
            0 => bytes[0] = b'x',
            1 => bytes[4] = 0x81,
            2 => bytes[7] = 33,
            3 => bytes[26..42].fill(0),
            4 => bytes[18..26].copy_from_slice(&((1_u64 << 41) | (15 << 36) | 1).to_be_bytes()),
            5 => bytes[18..26]
                .copy_from_slice(&((44100_u64 << 44) | (1 << 41) | (15 << 36)).to_be_bytes()),
            6 => bytes[18..26]
                .copy_from_slice(&((44100_u64 << 44) | (1 << 41) | (19 << 36) | 1).to_be_bytes()),
            7 => bytes[18..26]
                .copy_from_slice(&((44100_u64 << 44) | (2 << 41) | (15 << 36) | 1).to_be_bytes()),
            8 => bytes[18..26].copy_from_slice(
                &((44100_u64 << 44) | (1 << 41) | (15 << 36) | 0xf_ffff_ffff).to_be_bytes(),
            ),
            9 => bytes[18..26]
                .copy_from_slice(&((44100_u64 << 44) | (1 << 41) | (31 << 36) | 1).to_be_bytes()),
            _ => unreachable!(),
        }
        fs::write(&fixture.audio, bytes).unwrap();
        let marker = fixture.root.join("decoder-called");
        fixture.script(&fixture.ffmpeg, &format!("touch '{}'\n", marker.display()));
        assert_eq!(
            fixture.fingerprint().await,
            Err(AcoustidError::ProcessFailed),
            "variant {variant}"
        );
        assert!(!marker.exists());
    }
}

#[tokio::test]
async fn verified_pcm_only_runs_after_an_actual_failed_raw_exit() {
    let mut fixture = Fixture::new();
    fixture.script(&fixture.ffmpeg, "exit 99\n");
    fixture.script(&fixture.fpcalc, "printf '%s' '{\"fingerprint\":[4,5,6]}'\n");
    assert_eq!(fixture.fingerprint().await.unwrap(), [4, 5, 6]);
    fixture.script(
        &fixture.fpcalc,
        "printf '%s' '{\"duration\":10,\"fingerprint\":\"abc\"}'\n",
    );
    assert_eq!(
        fingerprint_file_with(
            &fixture.fpcalc,
            &fixture.audio,
            &AtomicBool::new(false),
            Duration::from_secs(3),
            1024,
            1024
        )
        .await
        .unwrap()
        .value,
        "abc"
    );
    fixture.script(&fixture.fpcalc, "printf '%s' '{\"fingerprint\":[]}'\n");
    assert_eq!(
        fixture.fingerprint().await,
        Err(AcoustidError::InvalidFingerprint)
    );
    fixture.script(&fixture.fpcalc, "exit 3\n");
    let wrong_extension = fixture.root.join("source.wav");
    fs::rename(&fixture.audio, &wrong_extension).unwrap();
    fixture.audio = wrong_extension;
    assert_eq!(
        fixture.fingerprint().await,
        Err(AcoustidError::ProcessFailed)
    );
}

#[tokio::test]
async fn verified_pcm_rejects_failed_tools_and_invalid_fingerprint_output() {
    for (output, status, error) in [
        (
            r#"{"fingerprint":[1,2,3]}"#,
            3,
            AcoustidError::ProcessFailed,
        ),
        (
            r#"{"fingerprint":[]}"#,
            0,
            AcoustidError::InvalidFingerprint,
        ),
        (
            r#"{"fingerprint":[-1]}"#,
            0,
            AcoustidError::InvalidFingerprint,
        ),
        ("not-json", 0, AcoustidError::InvalidFingerprint),
    ] {
        let fixture = Fixture::new();
        accept_any_pcm(&fixture, output, status);
        assert_eq!(fixture.fingerprint().await, Err(error));
    }
    let fixture = Fixture::new();
    fixture.script(&fixture.ffmpeg, "cat >/dev/null\nprintf test\nexit 3\n");
    assert_eq!(
        fixture.fingerprint().await,
        Err(AcoustidError::ProcessFailed)
    );
    fs::remove_file(&fixture.ffmpeg).unwrap();
    assert_eq!(
        fixture.fingerprint().await,
        Err(AcoustidError::ProcessFailed)
    );
}

#[tokio::test]
async fn verified_pcm_bounds_both_tool_errors_and_fingerprint_output() {
    let fixture = Fixture::new();
    let limits = FingerprintLimits {
        stdout: 64,
        stderr: 8,
    };
    fixture.script(
        &fixture.ffmpeg,
        "cat >/dev/null\nprintf test\nprintf '123456789' >&2\n",
    );
    assert_eq!(
        fixture
            .run(&AtomicBool::new(false), Duration::from_secs(3), limits)
            .await,
        Err(AcoustidError::OutputTooLarge)
    );
    fixture.script(&fixture.ffmpeg, "cat >/dev/null\nprintf test\n");
    fixture.script(
        &fixture.fpcalc,
        "if [ \"$5\" = -format ]; then cat >/dev/null; printf '%0100d' 0; else exit 3; fi\n",
    );
    assert_eq!(
        fixture
            .run(&AtomicBool::new(false), Duration::from_secs(3), limits)
            .await,
        Err(AcoustidError::OutputTooLarge)
    );
    fixture.script(&fixture.fpcalc, "if [ \"$5\" = -format ]; then cat >/dev/null; printf '%s' '{\"fingerprint\":[1,2,3]}'; printf '123456789' >&2; else exit 3; fi\n");
    assert_eq!(
        fixture
            .run(&AtomicBool::new(false), Duration::from_secs(3), limits)
            .await,
        Err(AcoustidError::OutputTooLarge)
    );
}

#[tokio::test]
async fn verified_pcm_rejects_source_mutation_and_replacement() {
    for replace in [false, true] {
        let fixture = Fixture::new();
        let mutation = if replace {
            format!(
                "cp '{0}' '{0}.new'; mv '{0}.new' '{0}'",
                fixture.audio.display()
            )
        } else {
            format!("printf x >> '{}'", fixture.audio.display())
        };
        fixture.script(
            &fixture.ffmpeg,
            &format!("cat >/dev/null\n{mutation}\nprintf test\n"),
        );
        assert_eq!(
            fixture.fingerprint().await,
            Err(AcoustidError::ProcessFailed)
        );
    }
}

fn assert_reaped(path: &Path) {
    let pid: i32 = fs::read_to_string(path).unwrap().trim().parse().unwrap();
    // These are the two directly spawned children, whose PID files were
    // observed before cancellation. An exited but unreaped child still exists.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[tokio::test]
async fn verified_pcm_cancellation_and_timeout_reap_both_children_and_release_the_permit() {
    for cancel in [false, true] {
        let fixture = Fixture::new();
        let decoder_pid = fixture.root.join("decoder.pid");
        let fingerprint_pid = fixture.root.join("fingerprint.pid");
        fixture.script(
            &fixture.ffmpeg,
            &format!("echo $$ > '{}'\nexec sleep 30\n", decoder_pid.display()),
        );
        fixture.script(&fixture.fpcalc, &format!("if [ \"$5\" = -format ]; then echo $$ > '{}'; exec cat >/dev/null; else exit 3; fi\n", fingerprint_pid.display()));
        let cancelled = AtomicBool::new(false);
        let (result, ()) = tokio::join!(
            fixture.run(
                &cancelled,
                Duration::from_secs(5),
                FingerprintLimits::default()
            ),
            async {
                if cancel {
                    tokio::time::timeout(Duration::from_secs(6), async {
                        while !decoder_pid.exists() || !fingerprint_pid.exists() {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .unwrap();
                    cancelled.store(true, Ordering::Release);
                }
            },
        );
        assert_eq!(
            result,
            Err(if cancel {
                AcoustidError::Cancelled
            } else {
                AcoustidError::ProcessTimedOut
            })
        );
        assert_reaped(&decoder_pid);
        assert_reaped(&fingerprint_pid);
    }
    // A leaked permit or detached pump would prevent subsequent work.
    assert_eq!(Fixture::new().fingerprint().await.unwrap(), [1, 2, 3]);
}

#[tokio::test]
async fn verified_pcm_shares_the_direct_attempt_deadline() {
    let fixture = Fixture::new();
    fixture.script(&fixture.fpcalc, "if [ \"$5\" = -format ]; then cat >/dev/null; printf '%s' '{\"fingerprint\":[1,2,3]}'; else sleep 0.6; exit 3; fi\n");
    fixture.script(&fixture.ffmpeg, "cat >/dev/null\nsleep 0.6\nprintf test\n");
    assert_eq!(
        fixture
            .run(
                &AtomicBool::new(false),
                Duration::from_secs(1),
                FingerprintLimits::default()
            )
            .await,
        Err(AcoustidError::ProcessTimedOut)
    );
}

#[tokio::test]
#[ignore = "requires an explicit copied damaged FLAC and independent PCM fingerprint reference"]
async fn verified_pcm_copied_real_flac_contract() {
    let source = PathBuf::from(std::env::var_os("SOUNDROBE_VERIFIED_FLAC_FIXTURE").unwrap());
    let reference = PathBuf::from(std::env::var_os("SOUNDROBE_VERIFIED_FLAC_REFERENCE").unwrap());
    let expected: serde_json::Value =
        serde_json::from_slice(&fs::read(reference).unwrap()).unwrap();
    let expected = parse_raw_output(&serde_json::to_vec(&expected).unwrap()).unwrap();
    let fixture = Fixture::new();
    let original = fs::read(&source).unwrap();
    fs::write(&fixture.audio, &original).unwrap();
    let executable = fpcalc_executable();
    let direct = std::process::Command::new(&executable)
        .args(["-json", "-raw", "-length", "0"])
        .arg(&fixture.audio)
        .output()
        .unwrap();
    assert!(
        !direct.status.success(),
        "the copied fixture must exercise recovery"
    );
    let cancelled = AtomicBool::new(false);
    assert_eq!(
        raw_fingerprint_file_with(&executable, &fixture.audio, &cancelled)
            .await
            .unwrap(),
        expected
    );
    assert_eq!(fs::read(&fixture.audio).unwrap(), original);
    let mut bad_checksum = original.clone();
    bad_checksum[26] ^= 1;
    fs::write(&fixture.audio, bad_checksum).unwrap();
    assert_eq!(
        raw_fingerprint_file_with(&executable, &fixture.audio, &cancelled).await,
        Err(AcoustidError::ProcessFailed)
    );
    fs::write(&fixture.audio, &original[..original.len() - 4096]).unwrap();
    assert_eq!(
        raw_fingerprint_file_with(&executable, &fixture.audio, &cancelled).await,
        Err(AcoustidError::ProcessFailed)
    );
    assert_eq!(fs::read(source).unwrap(), original);
}
