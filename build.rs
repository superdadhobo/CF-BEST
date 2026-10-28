fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/app-icon.ico");

    // 只在目标平台为 Windows 时嵌入图标与版本信息（交叉编译时按目标判断，而不是宿主）。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let macros = [
            format!("VER_MAJOR={}", env!("CARGO_PKG_VERSION_MAJOR")),
            format!("VER_MINOR={}", env!("CARGO_PKG_VERSION_MINOR")),
            format!("VER_PATCH={}", env!("CARGO_PKG_VERSION_PATCH")),
        ];
        embed_resource::compile("app.rc", &macros)
            .manifest_optional()
            .expect("编译 Windows 图标资源失败");
    }
}
