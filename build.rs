use std::process::Command;

fn main() {
    let version = std::env::var("ARBITRARY_ALMANAC_VERSION").unwrap_or_else(|_| {
        let date = cmd("date", &["-u", "+%Y-%m-%d"]);
        let change = cmd(
            "jj",
            &["log", "-r", "@", "--no-graph", "-T", "change_id.short()"],
        );
        format!("{date}+{change}-dev")
    });
    println!("cargo:rustc-env=ARBITRARY_ALMANAC_VERSION={version}");
}

fn cmd(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}
