use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::process::exit;
use std::sync::OnceLock;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

use anyhow::Result;
use zip::ZipArchive;
use rocket::{self, uri, Config, Response, Request};
use rocket::shield::{Shield, NoSniff};
use rocket::config::{LogLevel, TlsConfig, SecretKey};
use rocket::fs::NamedFile;
use rocket::response::{self, Redirect, Responder};
use rocket::http::{ContentType, Status};
use async_recursion::async_recursion;

use crate::{async_ptr_create, arc_pinned_ptr_create};
use crate::utils::{AsyncPtr, ArcPinnedPtr};
use crate::utils::{index_zip::index_zip_dir, index_callback::ZipCallback};

type ArcFileMapPtr = ArcPinnedPtr<BTreeMap<String, FileIndex>>;
type ArcZipHandleMapPtr = ArcPinnedPtr<BTreeMap<String, ZipArchive<BufReader<File>>>>;

struct GlobalControl {
	pub file_db: ArcFileMapPtr,
	pub zip_handles: ArcZipHandleMapPtr,
	pub landing_page: String,
	pub land_with_path: bool
}

static GLOBAL_CTRL: OnceLock<AsyncPtr<GlobalControl>> = OnceLock::new();
fn global() -> &'static AsyncPtr<GlobalControl> {
	GLOBAL_CTRL.get_or_init(|| async_ptr_create!(GlobalControl {
		file_db: arc_pinned_ptr_create!(BTreeMap::new()),
		zip_handles: arc_pinned_ptr_create!(BTreeMap::new()),
		landing_page: String::new(),
		land_with_path: false
	}))
}

// static FILE_DB: OnceLock<ArcFileMapPtr> = OnceLock::new();
// fn file_db() -> &'static ArcFileMapPtr {
// 	FILE_DB.get_or_init(|| arc_pinned_ptr_create!(BTreeMap::new()))
// }

// static ZIP_HANDLES: OnceLock<ArcZipHandleMapPtr> = OnceLock::new();
// fn global().zip_handles -> &'static ArcZipHandleMapPtr {
// 	ZIP_HANDLES.get_or_init(|| arc_pinned_ptr_create!(BTreeMap::new()))
// }

// static LANDING_PAGE: OnceLock<String> = OnceLock::new();
// fn landing_page(initial: &str) -> &'static String {
// 	LANDING_PAGE.get_or_init(|| String::from(initial))
// }

pub struct ServeOptions<'a> {
	pub host: &'a str,
	pub port: u16,
	pub use_ssl: bool,
	pub ssl_cert: Option<String>,
	pub ssl_key: Option<String>,
	pub landing: Option<String>,
	pub land_with_path: bool
}

pub struct IndexOptions {
	pub depth: isize,
	pub core_num: usize,
}

// (file_type, zip_file_path, zip_index)
//				^ This is the path to the zip file
//								^ This is the internal index of the file in the zip file
#[derive(Clone)]
struct FileIndex(u16, Option<String>, Option<usize>);

#[allow(unused)]
impl FileIndex {
	pub fn new(is_inside_zip: bool, is_dir: bool, zip_path: Option<String>, in_zip_path: Option<usize>) -> Self {
		Self(if is_inside_zip { 0x01 } else { 0 } | if is_dir { 0x10 } else { 0 }, zip_path, in_zip_path)
	}
	pub fn is_inside_zip(&self) -> bool { self.0 & 0x01 > 0 }
	pub fn is_dir(&self) -> bool { self.0 & 0x10 > 0 }
	pub fn is_file(&self) -> bool { self.0 & 0x10 == 0 }
}

#[async_recursion]
async fn iter_dir<F, R, 'a>(dir: &Path, depth: isize, cb: &mut F) -> Result<()>
where
	F: FnMut(PathBuf, PathBuf) -> R + Send,
	R: Future<Output = Result<()>> + Send, {
	if dir.is_dir() {
		for i in fs::read_dir(dir)? {
			let entry = i?;
			let entry_path = entry.path();
			cb(dir.to_path_buf(), entry_path.clone()).await?;
			if (depth > 0 || depth == -1) && entry_path.is_dir() {
				iter_dir(&entry_path, if depth == -1 { depth } else { depth - 1 }, cb).await?;
			}
		}
	}
	Ok(())
}

#[async_recursion]
async fn iter_dir_cb(dir: PathBuf, x: PathBuf) -> Result<()> {
	let zip_map;
	{
		let ctrl = global().lock().await;
		zip_map = ctrl.zip_handles.clone();
	}
	if let Some(ext) = x.extension() {
		if let Some(str) = ext.to_str() {
			if str == "zip" {
				zip_map.lock().unwrap().insert(
					x.to_str().unwrap().to_string(),
					ZipArchive::new(BufReader::new(File::open(x).unwrap())).unwrap());
				return Ok(());
			}
		}
	}
	let key = x.strip_prefix(dir).unwrap().to_str().unwrap().to_string();
	let key = key.replace('\\', "/");
	{
		let ctrl = global().lock().await;
		ctrl.file_db.lock().unwrap().entry(key).or_insert_with(|| FileIndex::new(false, x.is_dir(), None, None));
	}
	Ok(())
}

async fn create_file_db(dir: &str, index_options: &IndexOptions, file_db: ArcFileMapPtr) -> Result<()> {
	println!("[INFO] Creating file database...");

	let begin_time = Instant::now();
	let index_join_handle;
	{
		let file_db_clone = file_db.clone();
		let parent_dir = dir.to_string();
		index_join_handle = index_zip_dir(dir, index_options.core_num, index_options.depth, ZipCallback::new(move |x, i, f| {
			// Whoever comes first gets inserted first
			let xname = x.name();
			let parent_dir = Path::new(&parent_dir);
			let zip_file_dir = Path::new(&f);
			let mut relative_path_buf = zip_file_dir.strip_prefix(parent_dir).unwrap().to_path_buf();
			relative_path_buf.pop();
			relative_path_buf.push(if x.is_dir() { &xname[..xname.len() - 1] } else { xname });

			let file_path_str = relative_path_buf.to_string_lossy().replace('\\', "/");
			file_db_clone.lock().unwrap().insert(file_path_str, FileIndex::new(true, x.is_dir(), Some(f.to_string()), Some(i)));
		}));
	}

	iter_dir(Path::new(dir), index_options.depth, &mut iter_dir_cb).await?;
	// iter_dir(Path::new(dir), index_options.depth, || {}).await;

	index_join_handle.await?;
	println!("[INFO] File database created. Time: {}ms.", (Instant::now() - begin_time).as_millis());

	Ok(())
}

#[allow(dead_code)]
enum GetResponse {
    StringContent(ContentType, String),
    Bytes(ContentType, Vec<u8>),
    File(Option<NamedFile>),
	Error(Status)
}

macro_rules! response_build {
	($content_type:expr, $body:expr) => {
		rocket::Response::build()
			.header($content_type)
			.sized_body($body.len(), std::io::Cursor::new($body))
			.ok()
	};
}

impl<'a> Responder<'a, 'static> for GetResponse {
	fn respond_to(self, request: &'a Request<'_>) -> response::Result<'static> {
		match self {
			GetResponse::StringContent(content_type, body) => response_build!(content_type, body),
			GetResponse::Bytes(content_type, body) => response_build!(content_type, body),
			GetResponse::File(file_option) => {
				match file_option {
					Some(file) => file.respond_to(request),
					None => Response::build().status(Status::NotFound).ok()
				}
			},
			GetResponse::Error(status) => Response::build().status(status).ok()
		}
	}
}

async fn read_file_from_zip(zip_path: &String, zip_index: usize) -> Vec<u8> {
	let zip_handles;
	{
		let ctrl = global().lock().await;
		zip_handles = ctrl.zip_handles.clone();
	}
	let mut zip_handles = zip_handles.lock().unwrap();
	let zip_handle = zip_handles.get_mut(zip_path).unwrap();
	let mut zip_file = zip_handle.by_index(zip_index).unwrap();
	let mut vec = Vec::<u8>::with_capacity(zip_file.size() as usize);
	io::copy(&mut zip_file, &mut vec).unwrap();
	vec
}

fn count_occurrences(s: &str, c: char) -> usize {
    s.chars().filter(|&ch| ch == c).count()
}

fn insert_base_tag(data: &mut Vec<u8>, base_href: &str) {
	let mut content = String::from_utf8(data.clone()).unwrap();
    let base_tag = format!("<base href=\"{}\">", base_href);
    if let Some(head_end) = content.find("</head>") {
        content.insert_str(head_end, &base_tag);
        *data = content.into_bytes();
    }
}

macro_rules! response_file_index {
	($file_index_opt:expr, $file_ext:expr, $cur_path:expr, $auto_index:expr) => {
		if let Some(file_index) = $file_index_opt {
			match file_index.0 {
				0x00 => return GetResponse::File(NamedFile::open($cur_path).await.ok()),
				0x01 => {
					let zip_path = file_index.1.clone().unwrap();
					let zip_index = file_index.2.clone().unwrap();
					let ctype;
					if let Some(ext) = $file_ext {
						ctype = match ext.to_str().unwrap() {
							"html" => ContentType::HTML,
							"js" => ContentType::JavaScript,
							"css" => ContentType::CSS,
							_ => ContentType::Bytes
						}
					}
					else {
						ctype = ContentType::Bytes;
					}
					let mut data = read_file_from_zip(&zip_path, zip_index).await;
					if ctype == ContentType::HTML && $auto_index {
						insert_base_tag(&mut data, $cur_path);
					}
					return GetResponse::Bytes(ctype, data);
				},
				_ => {}
			}
		}
	};
}

#[allow(unused)]
#[derive(Responder)]
enum RouteResult {
	GetResponse(GetResponse),
	Redirect(Redirect),
	String(String)
}

#[allow(unused)]
fn html_redirect_str(uri: String) -> String {
	String::from(format!(r#"<html><head><meta http-equiv="refresh" content="0;url={}" /><title></title></head><body></body></html>"#, uri))
}

#[rocket::get("/")]
async fn landing_route() -> RouteResult {
	let landing_page;
	let land_with_path;
	{
		let ctrl = global().lock().await;
		landing_page = ctrl.landing_page.clone();
		land_with_path = ctrl.land_with_path;
	}
	if landing_page.is_empty() {
		return RouteResult::GetResponse(file_route(PathBuf::new()).await);
	}
	else {
		let a = landing_page.clone();
		if land_with_path {
			return RouteResult::Redirect(Redirect::temporary(uri!(file_route(PathBuf::from(a)))));
			// return RouteResult::String(html_redirect_str(uri!(file_route(PathBuf::from(a))).to_string()));
		}
		else {
			return RouteResult::GetResponse(file_route(PathBuf::from(&a[..])).await);
		}
	}
}

#[rocket::post("/<_path..>")]
async fn post_route(_path: PathBuf) -> &'static str {
	// Maybe some meilisearch integration in the future?
	"{}"
}

#[rocket::get("/<path..>")]
async fn file_route(path: PathBuf) -> GetResponse {
	let file_ext = path.extension();
	let cur_path = path.to_str().unwrap().replace('\\', "/");

	println!("[INFO] GET Request: {}", if cur_path.is_empty() { "current path" } else { &cur_path });

	let file_db;
	{
		let ctrl = global().lock().await;
		file_db = ctrl.file_db.clone();
	}
	let file_index_opt;
	let index_html_opt;
	{
		let file_db_lock = file_db.lock().unwrap();
		file_index_opt = file_db_lock.get(&cur_path).map(|f| f.clone());
		index_html_opt = file_db_lock.get(&format!("{}/index.html", cur_path)).map(|f| f.clone());
	}
	response_file_index!(index_html_opt, Some(&std::ffi::OsString::from("html")), &format!("{}/index.html", cur_path), true);
	response_file_index!(file_index_opt, file_ext, &cur_path, false);

	let mut file_list = vec![];
	for (k, _) in file_db.lock().unwrap().iter() {
		if k != &cur_path &&
		   k.starts_with(&cur_path) &&
		   count_occurrences(k.strip_prefix(&format!("{}/", cur_path)).unwrap_or(k), '/') == 0 {
			file_list.push(format!("<a href=\"\\{}\">{}</a>", k, k));
		}
	}
	GetResponse::StringContent(ContentType::HTML, format!("<pre>Files under {}:<br>  {}</pre>", if cur_path.is_empty() { "current path" } else { &cur_path }, file_list.join("<br>  ")))
}

pub async fn launch(dir: &str, index_options: &IndexOptions, serve_options: &ServeOptions<'_>) -> Result<()> {
	let current_path = PathBuf::from(dir);
	if !current_path.is_dir() {
		println!("[ERROR] Target path is not a directory!");
		exit(1);
	}

	let address = match serve_options.host.parse::<Ipv4Addr>() {
		Ok(x) => { IpAddr::V4(x) },
		Err(err) => {
			println!("[ERROR] Invalid IP address: {}, error: {}", serve_options.host, err);
			exit(1);
		}
	};

	let mut server_config = Config {
		log_level: LogLevel::Normal,
		cli_colors: false,
		address,
		port: serve_options.port,
		secret_key: SecretKey::generate().unwrap_or(SecretKey::from(&[0; 64])),
		..Default::default()
	};

	let file_db;
	{
		let mut ctrl = global().lock().await;
		file_db = ctrl.file_db.clone();

		if let Some(landing) = &serve_options.landing {
			ctrl.landing_page.clone_from(&landing);
			ctrl.land_with_path = serve_options.land_with_path;
			println!("[INFO] Serving default page: {}", landing);
		}
	}

	create_file_db(current_path.to_str().unwrap(), index_options, file_db).await?;

	if serve_options.use_ssl {
		server_config.tls = Some(TlsConfig::from_paths(
			serve_options.ssl_cert.clone().unwrap(),
			serve_options.ssl_key.clone().unwrap()
		));
	}
	
	#[cfg(not(debug_assertions))]
	set_log_level_critical(&mut server_config);

	let _ = rocket::custom(server_config)
		.attach(Shield::default().disable::<NoSniff>())
		.mount("/", rocket::routes![file_route])
		.mount("/", rocket::routes![post_route])
		.mount("/", rocket::routes![landing_route])
		.launch()
		.await?;

	Ok(())
}

#[inline]
#[allow(unused)]
fn set_log_level_critical(config: &mut Config) {
	config.log_level = LogLevel::Critical;
}
