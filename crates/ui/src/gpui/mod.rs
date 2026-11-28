//! Интерфейс на `gpui-kit`: оболочка с вкладками и страницы.

mod analysis;
mod analysis_details;
mod directories;
mod helpers;
mod home;
mod payment_documents;
mod prices;
mod quick_forms;
mod savings;
mod selectable;
mod settings;
mod shell;
mod support;
mod transactions;
mod transfers;

pub use shell::run;
