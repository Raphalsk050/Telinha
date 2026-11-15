//! Everything of Telinha that is not the window: the stores, the message envelope, the talk with
//! the message servers and with the capture engine. The window in `client/app` sits on top.

pub mod envelope;
pub mod text;
pub mod vault;
pub mod contacts;
pub mod disk;
pub mod servers;
pub mod avatars;
pub mod profile;
pub mod chats;
pub mod signaling;
pub mod applog;
pub mod ports;
pub mod sounds;
pub mod transfers;
pub mod hub;
pub mod engine;
pub mod updater;
pub mod linkpreview;
