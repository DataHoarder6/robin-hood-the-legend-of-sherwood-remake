//! Real-router end-to-end scenarios grouped by feature. The shared rig and
//! request builders live in `support`.
#![forbid(unsafe_code)]

robin_build_identity::define!();

mod boards;
mod storage;
mod submission;
mod support;
