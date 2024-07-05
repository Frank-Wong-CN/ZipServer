use std::fs::{self, File};
use std::path::Path;
use std::io::BufReader;
use std::time::Instant;

use anyhow::Result;
use zip::{ZipArchive, read::ZipFile};

use super::index_callback::ZipCallback;
use super::{ArcPinnedPtr, ArcPtr};
use crate::arc_pinned_ptr_create;

type BufZipReader = ZipArchive<BufReader<File>>;

pub fn index_zip_single_thread<T: FnMut(&ZipFile, usize, &str) + Send + 'static>(file: &str, cb: ArcPtr<ZipCallback<T>>) -> Result<u128> {
	let begin_time = Instant::now();
	let mut zip_file = ZipArchive::new(BufReader::new(File::open(file)?))?;
	for i in 0..zip_file.len() {
		cb.lock().unwrap().exec(&zip_file.by_index(i)?, i, file);
	}

	Ok((Instant::now() - begin_time).as_millis())
}

pub async fn index_zip_multi_thread<T: FnMut(&ZipFile, usize, &str) + Send + 'static>(file: &str, worker: usize, cb: ArcPtr<ZipCallback<T>>) -> Result<u128> {
	let begin_time = Instant::now();

	let zip_file = arc_pinned_ptr_create!(ZipArchive::new(BufReader::new(File::open(file)?))?);

	let mut file_len = zip_file.lock().unwrap().len();
	let thread_count = if file_len > worker { worker } else { file_len };
	let file_per_thread = file_len / thread_count;

	let mut join_handles = Vec::new();

	for i in 0..thread_count {
		join_handles.push(tokio::spawn(index_zip_mt_child(zip_file.clone(), file.to_string(), i, file_per_thread, if i == thread_count - 1 { file_len } else { file_per_thread }, cb.clone())));
		file_len -= file_per_thread;
	}

	for i in join_handles {
		if let Err(err) = i.await {
			println!("[ERROR] Error in thread: {}", err);
		}
	}

	Ok((Instant::now() - begin_time).as_millis())
}

async fn index_zip_mt_child<T: FnMut(&ZipFile, usize, &str) + Send + 'static>(zip_file: ArcPinnedPtr<BufZipReader>, zip_file_path: String, base_index: usize, file_per_thread:usize, how_much: usize, cb: ArcPtr<ZipCallback<T>>) -> Result<()> {
	for i in 0..how_much {
		let mut zip_ref = zip_file.lock().unwrap();
		let index = base_index * file_per_thread + i;
		cb.lock().unwrap().exec(&zip_ref.by_index(index)?, index, &zip_file_path);
	}
	Ok(())
}

fn iter_dir(dir: &Path, depth: isize, cb: &mut dyn FnMut(&Path)) -> Result<()> {
	if dir.is_dir() {
		for i in fs::read_dir(dir)? {
			let entry = i?;
			let entry_path = entry.path();
			if (depth > 0 || depth == -1) && entry_path.is_dir() {
				iter_dir(&entry_path, if depth == -1 { depth } else { depth - 1 }, cb)?;
			}
			else {
				cb(&entry_path);
			}
		}
	}
	Ok(())
}

pub async fn index_zip_dir<T: FnMut(&ZipFile, usize, &str) + Send + 'static>(dir: &str, worker: usize, depth: isize, cb: ArcPtr<ZipCallback<T>>) -> Result<u128> {
	let mut zip_files: Vec<String> = Vec::new();
	iter_dir(Path::new(dir), depth, &mut |x| {
		if x.extension().and_then(|x| { x.to_str() }) == Some("zip") {
			zip_files.push(String::from(x.to_str().unwrap()));
		}
	})?;

	let zip_files = arc_pinned_ptr_create!(zip_files);
	let begin_time = Instant::now();

	let mut join_handles = Vec::new();

	for _ in 0..worker {
		join_handles.push(tokio::spawn(index_zip_dir_child(zip_files.clone(), cb.clone())));
	}

	for i in join_handles {
		if let Ok(()) = i.await? {};
	}

	Ok((Instant::now() - begin_time).as_millis())
}

async fn index_zip_dir_child<T: FnMut(&ZipFile, usize, &str) + Send + 'static>(zip_files: ArcPinnedPtr<Vec<String>>, cb: ArcPtr<ZipCallback<T>>) -> Result<()> {
	'master: loop {
		let fname;
		'a: {
			if let Some(next_zip_file) = zip_files.lock().unwrap().pop() {
				fname = next_zip_file.clone();
			}
			else { break 'master; }
		}
		index_zip_single_thread(&fname, cb.clone())?;
	}
	Ok(())
}