#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl OpenCommand {
    pub fn for_platform(platform: &str, path: &str) -> Self {
        match platform {
            "macos" => Self {
                program: "open".into(),
                args: vec![path.into()],
            },
            "windows" => Self {
                program: "cmd".into(),
                args: vec!["/C".into(), "start".into(), "".into(), path.into()],
            },
            _ => Self {
                program: "xdg-open".into(),
                args: vec![path.into()],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_open() {
        let launcher = OpenCommand::for_platform("macos", "/tmp/demo.txt");
        assert_eq!(launcher.program, "open");
        assert_eq!(launcher.args, vec!["/tmp/demo.txt"]);
    }

    #[test]
    fn linux_uses_xdg_open() {
        let launcher = OpenCommand::for_platform("linux", "/tmp/demo.txt");
        assert_eq!(launcher.program, "xdg-open");
        assert_eq!(launcher.args, vec!["/tmp/demo.txt"]);
    }

    #[test]
    fn windows_uses_shell_open() {
        let launcher = OpenCommand::for_platform("windows", "C:\\temp\\demo.txt");
        assert_eq!(launcher.program, "cmd");
        assert_eq!(
            launcher.args,
            vec!["/C", "start", "", "C:\\temp\\demo.txt"]
        );
    }
}
