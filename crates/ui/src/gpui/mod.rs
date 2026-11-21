//! Интерфейс на `gpui-kit`: оболочка с вкладками и страницы.

mod analysis;
mod directories;
mod home;
mod payment_documents;
mod quick_forms;
mod savings;
mod settings;
mod shell;
mod support;
mod transactions;
mod transfers;

pub use shell::run;
