use std::collections::BTreeMap;
use std::process::exit;
use std::sync::Arc;
use std::path::PathBuf;
use std::fs::{self, File};
use std::io::{self, BufWriter, BufReader, Write};
use std::time::{Duration, Instant};

use crossbeam::channel::{self, Sender, Receiver};
use anyhow::Result;
use tokio::time::sleep;
use zip::write::FileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::arc_pinned_ptr_create;
use crate::utils::ArcPinnedPtr;
use crate::utils::index_callback::ZipCallback;
use crate::utils::index_zip::index_zip_single_thread;

pub struct SplitOptions<'a> {
	pub core_num: usize,
	pub channel_size: usize,
	pub thread_delay: usize,
	pub quiet: bool,
	pub verbose: bool,
	pub sort_by: &'a str
}

enum ControlCommand {
	FileSend(String, Vec<u8>),
	Shutdown
}

fn remove_target(input: &str, quiet: bool) {
	let meta = fs::metadata(input).unwrap();
	if meta.is_dir() {
		if quiet {
			println!("[WARN] Destination is a directroy. Removing because of the quiet flag.");
			if let Err(err) = fs::remove_dir_all(input) {
				println!("[ERROR] Cannot remove directory: {}", err);
				exit(1);
			}
		}
		else if let Err(err) = fs::remove_dir(input) {
			println!("[ERROR] Cannot remove directory: {}", err);
			println!("[INFO] Add \"--quiet\" to forcibly remove non-empty directories.");
			exit(1);
		}
	}
	else {
		if quiet { println!("[WARN] Destination already exists. Removing because of the quiet flag."); }
		if let Err(err) = fs::remove_file(input) {
			println!("[ERROR] Cannot remove file: {}", err);
			exit(1);
		}
	}
}

pub async fn split_archive_files(input: &str, output: &str, options: SplitOptions<'_>) {
	if let Err(err) = File::open(input) {
		println!("[ERROR] Cannot open file: {}", err);
		exit(1);
	}

	let SplitOptions { core_num, channel_size, thread_delay, quiet, verbose, sort_by } = options;

	if PathBuf::from(output).exists() {
		if quiet { remove_target(output, quiet); }
		else {
			print!("Target already exists. Overwrite? [y/N]: ");
			io::stdout().flush().unwrap();
			let mut buffer = String::with_capacity(255);
			if let Err(err) = io::stdin().read_line(&mut buffer) {
				println!("[ERROR] {}", err);
			}
			let buffer = buffer.trim().to_lowercase();
			match &buffer[..] {
				"y" | "yes" => { remove_target(output, quiet); },
				_ => exit(0)
			}
		}
	}

	if let Err(err) = fs::create_dir_all(output) {
		println!("[ERROR] Cannot create directory {}: {}", output, err);
		exit(1);
	}

	let begin = Instant::now();
	println!("[INFO] Indexing...");

	let file_map = arc_pinned_ptr_create!(BTreeMap::<String, usize>::new());
	file_indexer(input, file_map.clone(), sort_by).await;
	
	let (tx, rx) = channel::bounded::<ControlCommand>(channel_size);

	println!("[INFO] Spliting...");
	if verbose { println!("[VERBOSE] Sending file..."); }
	let sender_thread = file_sender(input, file_map, tx, core_num);

	let mut join_handles = vec![];
	for i in 0..core_num {
		join_handles.push(tokio::spawn(file_receiver(rx.clone(), input.to_string(), output.to_string(), i, verbose, thread_delay)));
	}

	if let Err(err) = sender_thread.await {
		println!("[ERROR] Thread communication error (Sender): {}", err);
		exit(1);
	}
	if verbose { println!("[VERBOSE] Send done."); }

	for i in join_handles {
		if let Err(err) = i.await {
			println!("[ERROR] Thread communication error (Receiver): {}", err);
			exit(1);
		}
	}

	println!("[INFO] Split completed! Time: {}ms.", (Instant::now() - begin).as_millis());
}

async fn file_indexer(input: &str, file_map: ArcPinnedPtr<BTreeMap<String, usize>>, sort_by: &str) {
	let file_map = Arc::downgrade(&file_map);
	let sort_by = String::from(sort_by);
	if let Err(err) = index_zip_single_thread(input, ZipCallback::new(move |x, i, _| {
		if let Some(file_map) = file_map.upgrade() {
			let sort_by = match &sort_by[..] {
				"name" => x.name().to_string(),
				"size" => format!("{}-{}", x.size(), x.name()),
				"time" => format!("{}-{}", x.last_modified().to_time().unwrap().unix_timestamp_nanos(), x.name()),
				_ => {
					println!("[ERROR] Sort field must be \"name\" or \"size\" or \"time\"");
					exit(1);
				}
			};
			file_map.lock().unwrap().insert(sort_by, i);
		}
	})) {
		println!("[ERROR] Index failed: {}", err);
		exit(1);
	}
}

async fn file_receiver(
	rx: Receiver<ControlCommand>,
	input: String,
	output: String,
	index: usize,
	verbose: bool,
	thread_delay: usize
) -> Result<()> {
	let mut path = PathBuf::from(output);
	match PathBuf::from(input).file_stem() {
		Some(fname) => path.push(format!("{}-{:03}.zip", fname.to_str().unwrap(), index)),
		None => path.push(format!("{:03}.zip", index))
	};
	if verbose { println!("[RECV {}] Thread initializing...", index); }
	if thread_delay > 0 { sleep(Duration::from_millis(thread_delay as u64)).await; }
	if verbose { println!("[RECV {}] Thread initialized.", index); }
	let mut archive_file = ZipWriter::new(BufWriter::new(File::create(path)?)); {
		loop {
			if let Ok(cmd) = rx.recv() {
				match cmd {
					ControlCommand::FileSend(fname, fcontent) => {
						if verbose { println!("[RECV {}] File {} received.", index, fname); }
						archive_file.start_file(fname, FileOptions::default())?;
						archive_file.write_all(&fcontent)?;
					},
					_ => { break; }
				}
			}
		}
	}
	if verbose { println!("[RECV {}] Thread done.", index); }
	archive_file.finish()?;

	Ok(())
}


async fn file_sender(
	input: &str,
	file_map: ArcPinnedPtr<BTreeMap<String, usize>>,
	tx: Sender<ControlCommand>,
	core_num: usize
) -> Result<()> {
	let mut archive_file = ZipArchive::new(BufReader::new(File::open(input)?))?;
	// let mut a = 0;
	for (_, i) in file_map.lock().unwrap().iter() {
		let zip_file = &mut archive_file.by_index(*i)?;
		let name = String::from(zip_file.name());
		let mut vec = Vec::<u8>::with_capacity(zip_file.size() as usize);
		io::copy(zip_file, &mut vec)?;
		tx.send(ControlCommand::FileSend(name, vec))?;
		// a += 1;
		// println!("[INFO] Split {} file(s).", a);
	}

	for _ in 0..core_num {
		tx.send(ControlCommand::Shutdown)?;
	}

	Ok(())
}