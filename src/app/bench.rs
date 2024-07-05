use std::fs;
use std::process::exit;

use crate::utils::index_zip::*;

macro_rules! call_index_statistic {
    ($func:ident, ($($args:expr),*), $update_func:ident, |$time:ident, $count:ident, $size:ident| $code:block) => {
		{
			let count = std::sync::Arc::new(std::sync::Mutex::new(std::pin::Pin::new(Box::new(0u64))));
			let size = std::sync::Arc::new(std::sync::Mutex::new(std::pin::Pin::new(Box::new(0u64))));

			let count_capture = std::sync::Arc::downgrade(&count);
			let size_capture = std::sync::Arc::downgrade(&size);

			let func_ret = $func($($args),*, crate::utils::index_callback::ZipCallback::new(move |x, _, _| {
				if let Some(count) = count_capture.upgrade() {
					**count.lock().unwrap() += 1;
				};
				if let Some(size) = size_capture.upgrade() {
					**size.lock().unwrap() += x.size();
				};
			}));

			let $update_func = |from: std::sync::Arc<std::sync::Mutex<std::pin::Pin<Box<u64>>>>| -> u64 {
				**from.lock().unwrap()
			};

			{
				let $count = count;
				let $size = size;
				let $time = func_ret;
				$code
			}
		}
    };
}

pub async fn read_dir(dir: &str, core_num: usize, depth: isize) {
	if let Err(file_err) = fs::read_dir(dir) {
		println!("[ERROR] Directory invalid: {}", file_err);
		exit(1);
	}

	call_index_statistic!(index_zip_dir, (dir, core_num, depth), update, |time_future, count, size| {
		match time_future.await {
			Ok(time) => {
				let count = update(count);
				let size = update(size);
				println!("[INFO] DIR iteration done.\n Time: {}ms\n File count: {}\n File size: {}", time, count, size);
			},
			Err(err) => { println!("[ERROR] {}", err); exit(1); }
		}
	});
}

pub async fn read_file(file: &str, core_num: usize) {
	if let Err(file_err) = fs::read(file) {
		println!("[ERROR] File invalid: {}", file_err);
		exit(1);
	}

	call_index_statistic!(index_zip_single_thread, (file), update, |time_result, count, size| {
		match time_result {
			Ok(time) => {
				let count = update(count);
				let size = update(size);
				println!("[INFO] ST iteration done.\n Time: {}ms\n File count: {}\n File size: {}", time, count, size);
			},
			Err(err) => { println!("[ERROR] {}", err); exit(1); }
		}
	});

	call_index_statistic!(index_zip_multi_thread, (file, core_num), update, |time_future, count, size| {
		match time_future.await {
			Ok(time) => {
				let count = update(count);
				let size = update(size);
				println!("[INFO] MT ({} threads) iteration done.\n Time: {}ms\n File count: {}\n File size: {}", core_num, time, count, size);
			},
			Err(err) => { println!("[ERROR] {}", err); exit(1); }
		}
	});
}