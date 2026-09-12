#include "telinha/transport/media_transport.hpp"

namespace tl::transport {

#if !defined(TELINHA_HAS_TRANSPORT_BACKENDS)

Result<std::unique_ptr<MediaTransport>> create_media_transport(const TransportConfig&)
{
    return Error{Status::NotImplemented, "create_media_transport: no backend linked"};
}

#endif

}  // namespace tl::transport
