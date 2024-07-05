use clap::{command, arg, Command};
use anyhow::Result;

mod utils;
mod app;

#[tokio::main]
async fn main() -> Result<()> {
	let matches = command!()
		.subcommand(
			Command::new("bench")
			.about("Benchmark")
			.arg(arg!(-f --file <FILE> "Open zip file").required_unless_present("dir").conflicts_with("dir"))
			.arg(arg!(-d --dir <DIR> "Open all zip files under a directory").required_unless_present("file").conflicts_with("file"))
			.arg(arg!(--depth <DEPTH> "How deep the iteration to subdirectories goes (-1 as infinite)").default_value("-1").conflicts_with("file"))
			.arg(arg!(-j --jobs <CORE_NUMBER> "How many thread to spawn").default_value("4"))
		)
		.subcommand(
			Command::new("split")
			.about("Split a zip file into several self-contained ones")
			.arg(arg!(-i --input <INPUT_FILES> "Open zip file").required(true))
			.arg(arg!(-o --output <OUTPUT_DIR> "Destnation directory").required(true))
			.arg(arg!(-j --jobs <CORE_NUMBER> "How many threads to spawn").default_value("4"))
			.arg(arg!(-c --chunks <CHUNK_NUMBER> "How many archives to split into (default is thread number)"))
			.arg(arg!(channel_size: --"channel-size" <CHANNEL_SIZE> "How many files to cache into the memory").default_value("512"))
			.arg(arg!(thread_delay: --"thread-delay" <THREAD_DELAY> "How many milliseconds to wait until the thread begins to write").default_value("0"))
			.arg(arg!(sort_by: --"sort-by" <SORT_FIELD> "Which field to sort against (name, time, size)").default_value("name"))
			.arg(arg!(unit_depth: --"unit-depth" <UNIT_PATH> "At what depth the subdirectory shall be regarded as a single unit to split"))
			.arg(arg!(-q --quiet "Overwrite file if exists"))
			.arg(arg!(-v --verbose "Verbose logging to terminal"))
		)
		.subcommand(
			Command::new("serve")
			.about("Serve zip files")
			.arg(arg!(dir: [DIRECTORY] "Starting directory").default_value("."))
			.arg(arg!(-d --depth <DEPTH> "How deep the zip files are indexeds (-1 as infinite)").default_value("-1"))
			.arg(arg!(-j --jobs <CORE_NUMBER> "How many thread to index the files").default_value("4"))
			.arg(arg!(-l --listen <LISTEN_HOST> "Listen host address").default_value("0.0.0.0"))
			.arg(arg!(-p --port <LISTEN_PORT> "Listen port").default_value("8192"))
			.arg(arg!(ssl_cert: --"ssl-cert" <SSL_CERT> "SSL certificate for TLS (optional, required if --ssl-key is set)").requires("ssl_key"))
			.arg(arg!(ssl_key: --"ssl-key" <SSL_KEY> "SSL key for TLS (optional, required if --ssl-cert is set)").requires("ssl_cert"))
			.arg(arg!(landing: --"landing-page" <PAGE_PATH> "The path to the landing page when getting the root route."))
			.arg(arg!(land_with_path: --"land-with-path" "Open landing page with full path").requires("landing"))
		)
		.get_matches();

	match matches.subcommand() {
		Some(("bench", arguments)) => { app::app_bench(arguments).await; },
		Some(("split", arguments)) => { app::app_split(arguments).await; },
		Some(("serve", arguments)) => { app::app_serve(arguments).await; },
		_ => { println!("[ERROR] Unrecognized command or subcommand. Run this program again with --help for more information."); }
	}

	Ok(())
}
