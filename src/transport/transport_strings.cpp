#include "telinha/transport/media_transport.hpp"

namespace tl::transport {

const char* to_string(TransportRole role) noexcept
{
    switch (role) {
        case TransportRole::Sender: return "Sender";
        case TransportRole::Receiver: return "Receiver";
    }
    return "Unrecognized";
}

const char* to_string(ConnectionState state) noexcept
{
    switch (state) {
        case ConnectionState::New: return "New";
        case ConnectionState::Gathering: return "Gathering";
        case ConnectionState::Connecting: return "Connecting";
        case ConnectionState::Connected: return "Connected";
        case ConnectionState::Disconnected: return "Disconnected";
        case ConnectionState::Failed: return "Failed";
        case ConnectionState::Closed: return "Closed";
    }
    return "Unrecognized";
}

const char* to_string(CandidatePolicy policy) noexcept
{
    switch (policy) {
        case CandidatePolicy::HostOnly: return "HostOnly";
        case CandidatePolicy::HostAndServerReflexive: return "HostAndServerReflexive";
        case CandidatePolicy::All: return "All";
    }
    return "Unrecognized";
}

}  // namespace tl::transport
