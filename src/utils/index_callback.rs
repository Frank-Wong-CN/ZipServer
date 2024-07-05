use zip::read::ZipFile;
use super::ArcPtr;
use crate::arc_ptr_create;

pub struct ZipCallback<T>
	where T: FnMut(&ZipFile, usize, &str) + Send + 'static {
		callback: Box<T>
}

#[allow(unused)]
impl<T> ZipCallback<T>
	where T: FnMut(&ZipFile, usize, &str) + Send + 'static {
		pub fn new(callback: T) -> ArcPtr<Self> {
			arc_ptr_create!(Self {
				callback: Box::new(callback)
			})
		}

		pub fn replace(&mut self, callback: T) {
			self.callback = Box::new(callback);
		}

		pub fn exec(&mut self, zip_file: &ZipFile, index: usize, zip_file_path: &str) {
			(self.callback)(zip_file, index, zip_file_path);
		}
}

#[macro_export]
macro_rules! noop_callback {
    () => {
        $crate::utils::index_callback::ZipCallback::new(|_| {})
    };
}
