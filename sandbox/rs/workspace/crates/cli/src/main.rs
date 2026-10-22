// Both crates are released together, so one version describes the pair.
fn main() {
    println!("sandbox-cli {}", env!("CARGO_PKG_VERSION"));
    println!("{}", sandbox_engine::greeting());
}
