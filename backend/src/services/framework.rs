use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct FrameworkInfo {
    pub framework: Option<String>,
    pub install_command: Option<String>,
    pub build_command: Option<String>,
    pub output_directory: Option<String>,
}

pub fn detect_framework(root: &Path) -> FrameworkInfo {
    let mut info = FrameworkInfo {
        install_command: Some("npm install".into()),
        ..Default::default()
    };

    if root.join("package.json").exists() {
        let pkg: Value = fs::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);

        let deps = pkg.get("dependencies").cloned().unwrap_or(Value::Null);
        let dev = pkg.get("devDependencies").cloned().unwrap_or(Value::Null);
        let has = |name: &str| deps.get(name).is_some() || dev.get(name).is_some();

        if has("next") {
            info.framework = Some("nextjs".into());
            info.build_command = Some("npm run build".into());
            info.output_directory = Some("out".into()); // static export if configured
            return info;
        }
        if has("vite")
            || root.join("vite.config.js").exists()
            || root.join("vite.config.ts").exists()
        {
            info.framework = Some("vite".into());
            info.build_command = Some("npm run build".into());
            info.output_directory = Some("dist".into());
            return info;
        }
        if has("react-scripts") {
            info.framework = Some("create-react-app".into());
            info.build_command = Some("npm run build".into());
            info.output_directory = Some("build".into());
            return info;
        }
        if has("nuxt") {
            info.framework = Some("nuxt".into());
            info.build_command = Some("npm run generate".into());
            info.output_directory = Some("dist".into());
            return info;
        }
        if has("@astrojs/check") || has("astro") || root.join("astro.config.mjs").exists() {
            info.framework = Some("astro".into());
            info.build_command = Some("npm run build".into());
            info.output_directory = Some("dist".into());
            return info;
        }
        if has("gatsby") {
            info.framework = Some("gatsby".into());
            info.build_command = Some("npm run build".into());
            info.output_directory = Some("public".into());
            return info;
        }
        // generic node
        let scripts = pkg.get("scripts").cloned().unwrap_or(Value::Null);
        if scripts.get("build").is_some() {
            info.framework = Some("nodejs".into());
            info.build_command = Some("npm run build".into());
            info.output_directory = Some("dist".into());
            return info;
        }
    }

    if root.join("index.html").exists() {
        info.framework = Some("static".into());
        info.install_command = None;
        info.build_command = None;
        info.output_directory = Some(".".into());
        return info;
    }

    if root.join("Cargo.toml").exists() {
        info.framework = Some("rust".into());
        info.install_command = None;
        info.build_command = Some("cargo build --release".into());
        info.output_directory = Some("target/release".into());
        return info;
    }

    info.framework = Some("unknown".into());
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detect_static() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("flare-fw-test-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut f = fs::File::create(dir.join("index.html")).unwrap();
        writeln!(f, "<html></html>").unwrap();
        let info = detect_framework(&dir);
        assert_eq!(info.framework.as_deref(), Some("static"));
        let _ = fs::remove_dir_all(&dir);
    }
}
