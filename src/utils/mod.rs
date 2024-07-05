#![allow(unused)]
pub mod index_zip;
pub mod index_callback;

use std::sync::Arc;
use std::pin::Pin;

pub type AsyncPtr<T> = Arc<tokio::sync::Mutex<T>>;

pub type ArcPtr<T> = Arc<std::sync::Mutex<T>>;
pub type ArcSharedPtr<T> = ArcPtr<Box<T>>;
pub type ArcPinnedPtr<T> = ArcPtr<Pin<Box<T>>>;

#[macro_export]
macro_rules! async_ptr_create {
	($expr:expr) => {
		std::sync::Arc::new(tokio::sync::Mutex::new($expr))
	};
}

#[macro_export]
macro_rules! arc_ptr_create {
	($expr:expr) => {
		std::sync::Arc::new(std::sync::Mutex::new($expr))
	};
}

#[macro_export]
macro_rules! arc_shared_ptr_create {
	($expr:expr) => {
		std::sync::Arc::new(std::sync::Mutex::new(Box::new($expr)))
	};
}

#[macro_export]
macro_rules! arc_pinned_ptr_create {
	($expr:expr) => {
		std::sync::Arc::new(std::sync::Mutex::new(std::pin::Pin::new(Box::new($expr))))
	};
}