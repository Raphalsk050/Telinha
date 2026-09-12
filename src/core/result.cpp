#include "telinha/core/result.hpp"

namespace tl {
const char* to_string(Status status) noexcept
{
    switch (status) {
        case Status::Ok: return "Ok";
        case Status::Unknown: return "Unknown";
        case Status::InvalidArgument: return "InvalidArgument";
        case Status::OutOfMemory: return "OutOfMemory";
        case Status::OutOfRange: return "OutOfRange";
        case Status::NotImplemented: return "NotImplemented";
        case Status::NotSupported: return "NotSupported";
        case Status::Unavailable: return "Unavailable";
        case Status::AlreadyExists: return "AlreadyExists";
        case Status::NotFound: return "NotFound";
        case Status::PermissionDenied: return "PermissionDenied";
        case Status::Timeout: return "Timeout";
        case Status::WouldBlock: return "WouldBlock";
        case Status::Empty: return "Empty";
        case Status::Full: return "Full";
        case Status::DeviceLost: return "DeviceLost";
        case Status::TargetGone: return "TargetGone";
        case Status::ConfigurationChanged: return "ConfigurationChanged";
        case Status::PlatformError: return "PlatformError";
    }
    return "Unrecognized";
}
}  // namespace tl
