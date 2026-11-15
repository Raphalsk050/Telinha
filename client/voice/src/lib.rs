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
