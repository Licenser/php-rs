extern crate cc;

use std::env;
use std::io::Write;
use std::path::Path;
use std::process::Command;

const PHP_VERSION: &'static str = concat!("php-", env!("CARGO_PKG_VERSION"));

/// println_stderr and run_command_or_fail are copied from rdkafka-sys
macro_rules! println_stderr(
    ($($arg:tt)*) => { {
        let r = writeln!(&mut ::std::io::stderr(), $($arg)*);
        r.expect("failed printing to stderr");
    } }
);

fn run_command_or_fail(dir: String, cmd: &str, args: &[&str]) {
    println_stderr!(
        "Running command: \"{} {}\" in dir: {}",
        cmd,
        args.join(" "),
        dir
    );
    let ret = Command::new(cmd).current_dir(dir).args(args).status();
    match ret.map(|status| (status.success(), status.code())) {
        Ok((true, _)) => return,
        Ok((false, Some(c))) => panic!("Command failed with error code {}", c),
        Ok((false, None)) => panic!("Command got killed"),
        Err(e) => panic!("Command failed with error: {}", e),
    }
}

fn target(path: &str) -> String {
    let osdir = env::var("PWD").unwrap();
    let pfx = match env::var("CARGO_TARGET_DIR") {
        Ok(d) => d,
        Err(_) => String::from("target"),
    };
    let profile = env::var("PROFILE").unwrap();
    format!("{}/{}/{}/native/{}", osdir, pfx, profile, path)
}

fn exists(path: &str) -> bool {
    Path::new(target(path).as_str()).exists()
}

fn main() {
    let default_link_static = true;
    let php_version = option_env!("PHP_VERSION").unwrap_or(PHP_VERSION);

    println!("cargo:rerun-if-env-changed=PHP_VERSION");

    let link_static = env::var_os("PHP_LINK_STATIC")
        .map(|_| true)
        .unwrap_or(default_link_static);

    if !exists("php-src/LICENSE") {
        println_stderr!("Setting up PHP {}", php_version);
        run_command_or_fail(
            target(""),
            "git",
            &[
                "clone",
                "https://github.com/php/php-src",
                format!("--branch={}", php_version).as_str(),
            ],
        );
        run_command_or_fail(
            target("php-src"),
            "sed",
            &[
                "-e",
                "s/void zend_signal_startup/ZEND_API void zend_signal_startup/g",
                "-ibk",
                "Zend/zend_signal.c",
                "Zend/zend_signal.h",
            ],
        );
        run_command_or_fail(target("php-src"), "./genfiles", &[]);
        run_command_or_fail(target("php-src"), "./buildconf", &["--force"]);
        run_command_or_fail(
            target("php-src"),
            "./configure",
            &[
                "--enable-debug",
                "--enable-embed=static",
                "--without-iconv",
                "--disable-libxml",
                "--disable-dom",
                "--disable-xml",
                "--enable-maintainer-zts",
                "--disable-simplexml",
                "--disable-xmlwriter",
                "--disable-xmlreader",
                "--without-pear",
            ],
        );
        run_command_or_fail(target("php-src"), "make", &[]);
    }

    let include_dir = target("php-src");
    let lib_dir = target("php-src/libs");

    let link_type = if link_static { "=static" } else { "" };

    println!("cargo:rustc-link-lib{}=php7", link_type);
    println!("cargo:rustc-link-search=native={}", lib_dir);

    cc::Build::new()
        .file("src/shim.c")
        .include(&include_dir)
        .include(&format!("{}/TSRM", include_dir))
        .include(&format!("{}/Zend", include_dir))
        .include(&format!("{}/main", include_dir))
        .compile("foo");
}
