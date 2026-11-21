//! Встраивает иконку и сведения о версии в исполняемый файл Windows.

fn main() {
    println!("cargo:rerun-if-changed=windows/finance-app.rc");
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("windows/finance-app.rc", embed_resource::NONE)
            .manifest_required()
            .expect("не удалось встроить ресурсы Windows");
    }
}
