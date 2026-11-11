//! A stand-in pgrust: tiny scripts that answer `--version` like pgrust and otherwise behave
//! like a server that never speaks SQL. Enough to exercise every non-SQL path of `up`.
#![allow(dead_code)]

use devtools_core::binary::{BinaryManager, InstallMeta};
use devtools_core::config::{parse_database_url, DevConfig, DevEnvironment};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn write_script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn fake_release(dir: &Path) {
    fs::create_dir_all(dir.join("share")).unwrap();
    fs::write(dir.join("share/postgres.bki"), b"bki").unwrap();
    write_script(
        &dir.join("postgres"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'pgrust 0.0-test (PostgreSQL 18.6 compatible)'; exit 0; fi\nexec sleep 60\n",
    );
    write_script(
        &dir.join("initdb"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'initdb (pgrust 0.0-test)'; exit 0; fi\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"-D\" ]; then mkdir -p \"$2\"; echo 18 > \"$2/PG_VERSION\"; fi\n  shift\ndone\n",
    );
}

pub fn meta() -> InstallMeta {
    InstallMeta {
        source_type: "prebuilt".into(),
        source_commit: None,
        source_dirty: false,
        build_profile: "release".into(),
        features: vec![],
    }
}

pub fn env_in(root: &Path, port: u16) -> DevEnvironment {
    let url = format!("postgres://cloud:pw@127.0.0.1:{port}/cloud");
    DevEnvironment {
        root: root.to_path_buf(),
        config: DevConfig::default(),
        conn: parse_database_url(&url).unwrap(),
        conn_source: "test".into(),
    }
}

/// An environment with the stand-in server installed under `<root>/.devtools/bin/pgrust`.
pub fn installed_env(root: &Path, port: u16) -> DevEnvironment {
    let env = env_in(root, port);
    let rel = root.join("release");
    fake_release(&rel);
    BinaryManager::new(env.install_dir(), env.root.clone())
        .install_from_dir(&rel, meta())
        .unwrap();
    env
}

pub fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}
