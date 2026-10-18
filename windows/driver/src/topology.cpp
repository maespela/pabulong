// Topology miniport: one microphone "jack" wired straight to the wave
// filter. The pin category makes Windows show the endpoint as a
// microphone; the device description from the INF names it "Pabulong Mic".
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#include "common.h"

static KSDATARANGE AnalogRanges[] = {
    {sizeof(KSDATARANGE), 0, 0, 0, STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO), STATICGUIDOF(KSDATAFORMAT_SUBTYPE_ANALOG), STATICGUIDOF(KSDATAFORMAT_SPECIFIER_NONE)},
};

static PKSDATARANGE AnalogRangePointers[] = {&AnalogRanges[0]};

static PCPIN_DESCRIPTOR TopologyPins[] = {
    // PABULONG_TOPO_PIN_MIC
    {0, 0, 0, NULL,
     {0, NULL, 0, NULL, SIZEOF_ARRAY(AnalogRangePointers), AnalogRangePointers, KSPIN_DATAFLOW_IN, KSPIN_COMMUNICATION_NONE, &KSNODETYPE_MICROPHONE, NULL, 0}},
    // PABULONG_TOPO_PIN_BRIDGE
    {0, 0, 0, NULL,
     {0, NULL, 0, NULL, SIZEOF_ARRAY(AnalogRangePointers), AnalogRangePointers, KSPIN_DATAFLOW_OUT, KSPIN_COMMUNICATION_NONE, &KSCATEGORY_AUDIO, NULL, 0}},
};

static PCCONNECTION_DESCRIPTOR TopologyConnections[] = {
    {PCFILTER_NODE, PABULONG_TOPO_PIN_MIC, PCFILTER_NODE, PABULONG_TOPO_PIN_BRIDGE},
};

static PCFILTER_DESCRIPTOR TopologyFilter = {
    0, NULL,
    sizeof(PCPIN_DESCRIPTOR), SIZEOF_ARRAY(TopologyPins), TopologyPins,
    sizeof(PCNODE_DESCRIPTOR), 0, NULL,
    SIZEOF_ARRAY(TopologyConnections), TopologyConnections,
    0, NULL,
};

class CPabulongTopology : public IMiniportTopology, public CUnknown {
public:
    DECLARE_STD_UNKNOWN();
    DEFINE_STD_CONSTRUCTOR(CPabulongTopology);
    ~CPabulongTopology() {}
    IMP_IMiniportTopology;
};

STDMETHODIMP_(NTSTATUS) CPabulongTopology::NonDelegatingQueryInterface(_In_ REFIID iid, _COM_Outptr_ PVOID* object)
{
    if (IsEqualGUIDAligned(iid, IID_IUnknown)) {
        *object = PVOID(PUNKNOWN(PMINIPORTTOPOLOGY(this)));
    } else if (IsEqualGUIDAligned(iid, IID_IMiniport)) {
        *object = PVOID(PMINIPORT(this));
    } else if (IsEqualGUIDAligned(iid, IID_IMiniportTopology)) {
        *object = PVOID(PMINIPORTTOPOLOGY(this));
    } else {
        *object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    PUNKNOWN(*object)->AddRef();
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongTopology::Init(_In_ PUNKNOWN, _In_ PRESOURCELIST, _In_ PPORTTOPOLOGY)
{
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongTopology::GetDescription(_Out_ PPCFILTER_DESCRIPTOR* description)
{
    *description = &TopologyFilter;
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongTopology::DataRangeIntersection(_In_ ULONG PinId, _In_ PKSDATARANGE DataRange, _In_ PKSDATARANGE MatchingDataRange,
                                                             _In_ ULONG OutputBufferLength,
                                                             _Out_writes_bytes_to_opt_(OutputBufferLength, *ResultantFormatLength) PVOID ResultantFormat,
                                                             _Out_ PULONG ResultantFormatLength)
{
    UNREFERENCED_PARAMETER(PinId);
    UNREFERENCED_PARAMETER(DataRange);
    UNREFERENCED_PARAMETER(MatchingDataRange);
    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(ResultantFormat);
    UNREFERENCED_PARAMETER(ResultantFormatLength);
    // PortCls' default intersection handles our fixed ranges.
    return STATUS_NOT_IMPLEMENTED;
}

NTSTATUS CreateTopologyMiniport(PUNKNOWN* unknown)
{
    CPabulongTopology* topology = new (POOL_FLAG_NON_PAGED, PABULONG_POOLTAG) CPabulongTopology(NULL);
    if (topology == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    *unknown = PUNKNOWN(PMINIPORTTOPOLOGY(topology));
    (*unknown)->AddRef();
    return STATUS_SUCCESS;
}
