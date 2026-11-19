//! Real OS-process tests for the one canonical live RT1 runtime lease.
use super::*;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHILD_TEST: &str = "persona_store_lease::tests::child_runtime_lease_probe";

#[test]
fn child_runtime_lease_probe() {
    let Some(mode) = std::env::var_os("VPR_TEST_LEASE_MODE") else {
        return;
    };
    match mode.to_string_lossy().as_ref() {
        "holder" => {
            let lease = acquire_persona_store_lease().expect("holder owns OS lock");
            require_live_runtime_owner().expect("holder must have session authority");
            let marker = std::env::var_os("VPR_TEST_LEASE_MARKER").unwrap();
            fs::write(PathBuf::from(marker), b"ready").unwrap();
            std::thread::sleep(Duration::from_secs(4));
            drop(lease);
            assert!(require_live_runtime_owner().is_err());
        }
        "contender" => {
            assert!(require_live_runtime_owner().is_err());
            assert!(acquire_persona_store_lease().is_err());
            assert!(require_live_runtime_owner().is_err());
        }
        "successor" => {
            let lease = acquire_persona_store_lease().expect("released OS lock must be reusable");
            require_live_runtime_owner().expect("successor must own live runtime");
            drop(lease);
            assert!(require_live_runtime_owner().is_err());
        }
        _ => panic!("unexpected lease test mode"),
    }
}

#[test]
fn only_process_holding_canonical_owner_lease_can_admit_live_visitors() {
    let base = std::env::temp_dir().join(format!(
        "vpr-rt1-live-owner-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&base).unwrap();
    let path = base.join("persona.json");
    let marker = base.join("ready");
    let exe = std::env::current_exe().unwrap();
    let mut holder = Command::new(&exe)
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env("VPR_OWNER_LAB_PERSONA_STORE_PATH", &path)
        .env("VPR_TEST_LEASE_MODE", "holder")
        .env("VPR_TEST_LEASE_MARKER", &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        if holder.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let acquired = marker.exists();
    let result = if acquired {
        Command::new(&exe)
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env("VPR_OWNER_LAB_PERSONA_STORE_PATH", &path)
            .env("VPR_TEST_LEASE_MODE", "contender")
            .output()
            .unwrap()
    } else {
        let _ = holder.kill();
        let _ = holder.wait();
        panic!("holder child failed to acquire live runtime lock");
    };
    let _ = holder.kill(); // Crash recovery: the OS releases the owner lock.
    let _ = holder.wait();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout).contains("test result: ok."),
        "child contender test did not actually execute");
    let successor = Command::new(&exe)
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env("VPR_OWNER_LAB_PERSONA_STORE_PATH", &path)
        .env("VPR_TEST_LEASE_MODE", "successor")
        .output().unwrap();
    assert!(successor.status.success(), "{}", String::from_utf8_lossy(&successor.stderr));
    assert!(String::from_utf8_lossy(&successor.stdout).contains("test result: ok."));
    let _ = fs::remove_file(&marker);
    let _ = fs::remove_file(base.join("persona.json.owner.lock"));
    let _ = fs::remove_dir(&base);
}
