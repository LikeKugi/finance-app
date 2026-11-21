//! Встраивает иконку и сведения о версии в исполняемый файл Windows.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=windows/finance-app.rc");
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = env::var("CARGO_PKG_VERSION").expect("версия пакета Cargo не задана");
        let components: Vec<u16> = version
            .split('.')
            .map(|component| {
                component.parse().unwrap_or_else(|_| {
                    panic!("неподдерживаемая версия Windows-ресурса: {version}")
                })
            })
            .collect();
        if components.len() != 3 {
            panic!("версия Windows-ресурса должна иметь формат MAJOR.MINOR.PATCH: {version}");
        }

        let file_version = format!("{},{},{},0", components[0], components[1], components[2]);
        let icon_path =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR не задан"))
                .join("../../assets/icons/icon.ico")
                .canonicalize()
                .expect("не удалось найти иконку Windows")
                .to_string_lossy()
                .replace('\\', "/");
        let resource = fs::read_to_string("windows/finance-app.rc")
            .expect("не удалось прочитать шаблон Windows-ресурса")
            .replace("@FILE_VERSION@", &file_version)
            .replace("@VERSION@", &version)
            .replace("@ICON_PATH@", &icon_path);
        let resource_path =
            PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR не задан")).join("finance-app.rc");
        fs::write(&resource_path, resource).expect("не удалось создать Windows-ресурс");

        embed_resource::compile(resource_path, embed_resource::NONE)
            .manifest_required()
            .expect("не удалось встроить ресурсы Windows");
    }
}
