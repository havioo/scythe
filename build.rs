use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=assets/scythe.ico");
    println!("cargo:rerun-if-changed=assets/scythe.manifest");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = version.split('.').collect::<Vec<_>>().join(",") + ",0";
    let icon = root
        .join("assets/scythe.ico")
        .display()
        .to_string()
        .replace('\\', "/");
    let manifest = root
        .join("assets/scythe.manifest")
        .display()
        .to_string()
        .replace('\\', "/");
    let resource = format!(
        r#"
1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904b0"
  BEGIN
   VALUE "CompanyName", "havioo\0"
   VALUE "FileDescription", "Scythe headset control\0"
   VALUE "FileVersion", "{version}\0"
   VALUE "InternalName", "scythe\0"
   VALUE "LegalCopyright", "Copyright 2026 havioo. MIT License.\0"
   VALUE "OriginalFilename", "Scythe.exe\0"
   VALUE "ProductName", "Scythe\0"
   VALUE "ProductVersion", "{version}\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#
    );
    let script = output.join("scythe.rc");
    let compiled = output.join("scythe.res");
    fs::write(&script, resource).unwrap();
    let mut candidates = Vec::new();
    if let Some(rc) = env::var_os("RC") {
        candidates.push(PathBuf::from(rc));
    }
    if let Some(bin) = env::var_os("WindowsSdkVerBinPath") {
        candidates.push(PathBuf::from(bin).join("x64/rc.exe"));
    }
    if let Some(programs) = env::var_os("ProgramFiles(x86)") {
        let kits = PathBuf::from(programs).join("Windows Kits/10/bin");
        let mut versions: Vec<_> = fs::read_dir(&kits)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .collect();
        versions.sort_by_key(|e| {
            e.file_name()
                .to_string_lossy()
                .split('.')
                .filter_map(|n| n.parse::<u32>().ok())
                .collect::<Vec<_>>()
        });
        for entry in versions.into_iter().rev() {
            candidates.push(entry.path().join("x64/rc.exe"));
        }
    }
    candidates.push(PathBuf::from("rc.exe"));
    let compiler = candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("rc.exe"));
    let result = Command::new(&compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled)
        .arg(&script)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "Windows SDK resource compiler required ({}): {e}",
                compiler.display()
            )
        });
    assert!(result.success(), "Scythe resource compilation failed");
    println!("cargo:rustc-link-arg={}", compiled.display());
}
