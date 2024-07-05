use clap::ArgMatches;

mod bench;
mod split;
mod serve;

pub async fn app_bench(arguments: &ArgMatches) {
	let depth = arguments.get_one::<String>("depth").unwrap().trim().parse::<isize>().unwrap();
	let core_num = arguments.get_one::<String>("jobs").unwrap().trim().parse::<usize>().unwrap();

	if let Ok(Some(file)) = arguments.try_get_one::<String>("file") {
		println!("[INFO] Running benchmark on file {} under both ST and MT ({} threads) scenario.", file, core_num);
		bench::read_file(file, core_num).await;
	}

	if let Ok(Some(dir)) = arguments.try_get_one::<String>("dir") {
		println!("[INFO] Running benchmark on directory {} with {} threads.", dir, core_num);
		bench::read_dir(dir, core_num, depth).await;
	}
}

pub async fn app_split(arguments: &ArgMatches) {
	let input_zip = arguments.get_one::<String>("input").unwrap();
	let output_zip = arguments.get_one::<String>("output").unwrap();
	let core_num = arguments.get_one::<String>("jobs").unwrap().trim().parse::<usize>().unwrap();
	let channel_size = arguments.get_one::<String>("channel_size").unwrap().trim().parse::<usize>().unwrap();
	let thread_delay = arguments.get_one::<String>("thread_delay").unwrap().trim().parse::<usize>().unwrap();
	let sort_by = arguments.get_one::<String>("sort_by").unwrap();
	let quiet = arguments.get_flag("quiet");
	let verbose = arguments.get_flag("verbose");

	println!("[INFO] Split file {} to {} into {} parts.", input_zip, output_zip, core_num);

	split::split_archive_files(input_zip, output_zip, split::SplitOptions { core_num, channel_size, thread_delay, quiet, verbose, sort_by }).await;
}

pub async fn app_serve(arguments: &ArgMatches) {
	let dir = arguments.get_one::<String>("dir").unwrap();
	let depth = arguments.get_one::<String>("depth").unwrap().trim().parse::<isize>().unwrap();
	let core_num = arguments.get_one::<String>("jobs").unwrap().trim().parse::<usize>().unwrap();
	let host = arguments.get_one::<String>("listen").unwrap();
	let port = arguments.get_one::<String>("port").unwrap().trim().parse::<u16>().unwrap();

	let mut use_ssl = false;
	let ssl_cert;
	let ssl_key;

	if let Some(cert_path) = arguments.get_one::<String>("ssl_cert") {
		use_ssl = true;
		ssl_cert = Some(String::from(cert_path));
		ssl_key = Some(String::from(arguments.get_one::<String>("ssl_key").unwrap()));
	}
	else {
		ssl_cert = None;
		ssl_key = None;
	}

	let landing = arguments.get_one::<String>("landing").map(|x| x.clone());
	let land_with_path = arguments.get_flag("land_with_path");

	println!("[INFO] Serving file under {}. Listening http{}://{}:{}.", if dir == "." { "current directory" } else { dir }, if use_ssl { "s" } else { "" }, host, port);
	// println!("[INFO] Indexing subdirectories with a depth of {} and a thread number of {}.", depth, core_num);

	let index_options = serve::IndexOptions {
		depth, core_num
	};

	let serve_options = serve::ServeOptions {
		host, port, use_ssl, ssl_cert, ssl_key, landing, land_with_path
	};

	if let Err(err) = serve::launch(dir, &index_options, &serve_options).await {
		println!("[ERROR] File server error: {}", err);
	}
}
