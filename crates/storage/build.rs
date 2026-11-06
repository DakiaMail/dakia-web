fn main() {
    // sqlx::migrate! embeds directory contents; rebuild when migrations are added.
    println!("cargo:rerun-if-changed=migrations");
}
