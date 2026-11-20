// Migracje są wkompilowane makrem `sqlx::migrate!`; bez tego nowy plik w
// `migrations/` nie wymusiłby przebudowy binarek.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
