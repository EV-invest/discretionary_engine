use clap::{Args, CommandFactory};
use clap_complete::Shell as ClapShell;
use derive_more::derive::{Display, FromStr};

use crate::{Cli, config::EXE_NAME};

#[derive(Args, Clone, Debug)]
pub struct ShellInitArgs {
	shell: Shell,
}

pub fn output(args: ShellInitArgs) {
	let shell = args.shell;
	let s = format!(
		r#"{}
{}"#,
		shell.aliases(EXE_NAME),
		shell.completions(),
	);

	println!("{s}");
}
#[derive(Clone, Copy, Debug, Display, FromStr)]
enum Shell {
	Dash,
	Bash,
	Zsh,
	Fish,
}

impl Shell {
	fn aliases(&self, exe_name: &str) -> String {
		format!(
			r#"
alias de="{exe_name}"
"#
		)
	}

	fn to_clap_shell(self) -> ClapShell {
		match self {
			Shell::Dash => ClapShell::Bash,
			Shell::Bash => ClapShell::Bash,
			Shell::Zsh => ClapShell::Zsh,
			Shell::Fish => ClapShell::Fish,
		}
	}

	fn completions(&self) -> String {
		let mut cmd = Cli::command();
		let mut buffer = Vec::default();
		let shell = self.to_clap_shell();
		clap_complete::generate(shell, &mut cmd, EXE_NAME, &mut buffer);

		String::from_utf8(buffer).unwrap_or_else(|_| String::from("# Failed to generate completions"))
	}
}
