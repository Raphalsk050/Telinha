#include "telinha/diag/diagnostics.hpp"

#include "module_probes.hpp"
#include "platform_probes.hpp"
#include "report_builder.hpp"
#include "stun_probe.hpp"

namespace tl::diag {
namespace {
constexpr StunServer kDefaultStunServers[] = {
    {"stun.l.google.com", 19302},
    {"stun.cloudflare.com", 3478},
};

void resolve_verdicts(DiagnosticReport& report, bool network_probed) noexcept
{
    for (std::size_t i = 0; i < kDiagCategoryCount; ++i) {
        std::uint32_t total = 0;
        std::uint32_t healthy = 0;

        for (std::uint32_t item = 0; item < report.item_count; ++item) {
            if (static_cast<std::size_t>(report.items[item].category) != i) {
                continue;
            }
            ++total;
            if (report.items[item].available) {
                ++healthy;
            }
        }

        const auto category = static_cast<DiagCategory>(i);

        if (category == DiagCategory::Network && !network_probed) {
            report.verdicts[i] = DiagVerdict::NotProbed;
            continue;
        }

        if (total == 0) {
            report.verdicts[i] = DiagVerdict::NotProbed;
            continue;
        }

        if (healthy == total) {
            report.verdicts[i] = DiagVerdict::Ready;
            continue;
        }

        if (healthy == 0) {
            report.verdicts[i] = DiagVerdict::Blocked;
            continue;
        }

        report.verdicts[i] = DiagVerdict::Degraded;
    }
}

void record_network(ReportBuilder& builder, const NetworkProbeResult& result,
                    const Outcome& outcome) noexcept
{
    DiagItem* item = builder.add(DiagCategory::Network, "nat mapping");
    if (item == nullptr) {
        return;
    }

    item->status = outcome.ok() ? Status::Ok : outcome.error().status;
    item->platform_code = outcome.error().platform_code;
    item->available = result.mapping == NatMapping::EndpointIndependent;

    switch (result.mapping) {
        case NatMapping::EndpointIndependent:
            write_text(item->detail, kDiagDetailCapacity,
                       "same public port to both servers, direct connection should hold at ");
            append_text(item->detail, kDiagDetailCapacity, result.public_address);
            break;
        case NatMapping::AddressDependent:
            write_text(item->detail, kDiagDetailCapacity,
                       "public port changed per destination, symmetric nat, needs turn (");
            append_unsigned(item->detail, kDiagDetailCapacity, result.first_public_port);
            append_text(item->detail, kDiagDetailCapacity, " then ");
            append_unsigned(item->detail, kDiagDetailCapacity, result.second_public_port);
            append_text(item->detail, kDiagDetailCapacity, ")");
            break;
        case NatMapping::Blocked:
            write_text(item->detail, kDiagDetailCapacity,
                       "no stun answer, outbound udp is blocked");
            break;
        case NatMapping::Unknown:
            write_text(item->detail, kDiagDetailCapacity, outcome.error().context);
            break;
    }

    DiagItem* reflexive = builder.add(DiagCategory::Network, "public address");
    if (reflexive == nullptr) {
        return;
    }

    reflexive->available = result.servers_answered != 0;
    reflexive->status = reflexive->available ? Status::Ok : Status::Unavailable;
    if (reflexive->available) {
        write_text(reflexive->detail, kDiagDetailCapacity, result.public_address);
        append_text(reflexive->detail, kDiagDetailCapacity, ", answered by ");
        append_unsigned(reflexive->detail, kDiagDetailCapacity, result.servers_answered);
        append_text(reflexive->detail, kDiagDetailCapacity, " server(s)");
    } else {
        write_text(reflexive->detail, kDiagDetailCapacity, "not discovered");
    }
}
}  // namespace

const char* to_string(DiagCategory category) noexcept
{
    switch (category) {
        case DiagCategory::System: return "system";
        case DiagCategory::Graphics: return "graphics";
        case DiagCategory::Capture: return "capture";
        case DiagCategory::Encode: return "encode";
        case DiagCategory::Audio: return "audio";
        case DiagCategory::Network: return "network";
    }
    return "unknown";
}

const char* to_string(DiagVerdict verdict) noexcept
{
    switch (verdict) {
        case DiagVerdict::Ready: return "ready";
        case DiagVerdict::Degraded: return "degraded";
        case DiagVerdict::Blocked: return "blocked";
        case DiagVerdict::NotProbed: return "not probed";
    }
    return "unknown";
}

const char* to_string(NatMapping mapping) noexcept
{
    switch (mapping) {
        case NatMapping::Unknown: return "unknown";
        case NatMapping::Blocked: return "blocked";
        case NatMapping::EndpointIndependent: return "endpoint independent";
        case NatMapping::AddressDependent: return "address dependent";
    }
    return "unknown";
}

Span<const StunServer> default_stun_servers() noexcept
{
    return Span<const StunServer>{kDefaultStunServers,
                                  sizeof(kDefaultStunServers) / sizeof(kDefaultStunServers[0])};
}

Outcome collect_diagnostics(DiagnosticReport& out) noexcept
{
    return collect_diagnostics(out, DiagnosticOptions{});
}

Outcome collect_diagnostics(DiagnosticReport& out, const DiagnosticOptions& options) noexcept
{
    out = DiagnosticReport{};

    ReportBuilder builder(out);

    probe_system(builder);
    probe_graphics(builder);
    probe_capture(builder);
    probe_encode(builder);
    probe_audio(builder);

    if (options.probe_network) {
        Span<const StunServer> servers =
            options.stun_servers.empty() ? default_stun_servers() : options.stun_servers;

        const Outcome outcome = probe_nat_mapping(servers, options.stun_timeout_ms, out.network);
        record_network(builder, out.network, outcome);
    }

    resolve_verdicts(out, options.probe_network);
    return ok();
}
}  // namespace tl::diag
