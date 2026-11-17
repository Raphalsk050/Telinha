//! Voice calls: one WebRTC connection to each person in the room, the microphone going out and
//! everyone else's voice coming in.

pub mod audio;
pub mod call;
pub mod camera;
pub mod filelink;
pub mod soundboard;
pub mod tones;
pub mod video;

pub use call::{Call, CallEvent, CallStats, Participant, PeerState, SELF_STREAM, Sound, StreamStats, default_ice_servers};
pub use libwebrtc::peer_connection_factory::IceServer;

use libwebrtc::peer_connection_factory::PeerConnectionFactory;

/// The one WebRTC factory of the app, shared by every call and file link. Each factory opens its
/// own audio device module, and a second one in the same process left the next call silent:
/// after leaving a room and joining again, the others no longer heard this app.
pub(crate) fn factory() -> PeerConnectionFactory {
    static FACTORY: std::sync::Mutex<Option<PeerConnectionFactory>> = std::sync::Mutex::new(None);
    FACTORY.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_with(PeerConnectionFactory::default).clone()
}

#[cfg(test)]
mod tests {
    use libwebrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};

    #[test]
    fn a_connection_can_be_created() {
        let factory = PeerConnectionFactory::default();
        let connection = factory.create_peer_connection(RtcConfiguration::default());
        assert!(connection.is_ok());
    }
}
