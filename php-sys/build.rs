//extern crate bindgen;
extern crate cc;
extern crate num_cpus;

use std::env;
use std::io::Write;
use std::path::Path;
//use std::path::PathBuf;
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
    let cpus = format!("{}", num_cpus::get());
    let php_version = option_env!("PHP_VERSION").unwrap_or(PHP_VERSION);

    println!("cargo:rerun-if-env-changed=PHP_VERSION");

    if !exists("php-src") {
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
        if !exists("php-src/libs/libphp7.a") {
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
                    "--prefix=/usr/local/musl",
                    "--static",
                    "CC=musl-gcc",
                ],
            );
            run_command_or_fail(target("php-src"), "make", &["-j", cpus.as_str()]);
        }
    }

    let include_dir = target("php-src");
    let lib_dir = target("php-src/libs");

    println!("cargo:rustc-link-lib=static=php7");
    println!("cargo:rustc-link-search=native={}", lib_dir);

    /*
    let includes = ["/", "/TSRM", "/Zend", "/main"]
        .iter()
        .map(|d| format!("-I{}{}", include_dir, d))
        .collect::<Vec<String>>();

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_args(includes)
        .blacklist_type("FP_NAN")
        .blacklist_type("FP_INFINITE")
        .blacklist_type("FP_ZERO")
        .blacklist_type("FP_SUBNORMAL")
        .blacklist_type("FP_NORMAL")
        .blacklist_type("max_align_t")
        .derive_default(true)
        .generate()
        .expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
     */
    cc::Build::new()
        .file("src/shim.c")
        .include(&include_dir)
        .include(&format!("{}/TSRM", include_dir))
        .include(&format!("{}/Zend", include_dir))
        .include(&format!("{}/main", include_dir))
        .compile("phpshim");
}
