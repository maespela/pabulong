// WaveRT capture miniport: one 48 kHz mono capture pin whose data comes
// from the shared ring.
//
// There is no hardware clock, so the stream runs on the QPC: a 1 ms
// high-resolution timer (and every GetPosition call) works out how many
// frames "the device" has captured since KSSTATE_RUN, copies that many from
// the ring into the cyclic buffer the audio engine reads, and signals the
// engine's notification events at each packet boundary. Like the macOS HAL
// plug-in, it never waits for the app: an empty ring plays silence.
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#include "common.h"
#include "ring_read.h"

// ------------------------------------------------------------- tables

static KSDATARANGE_AUDIO HostRanges[] = {
    {   // 32-bit float, the format the app produces.
        {sizeof(KSDATARANGE_AUDIO), 0, 0, 0, STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO), STATICGUIDOF(KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
         STATICGUIDOF(KSDATAFORMAT_SPECIFIER_WAVEFORMATEX)},
        PABULONG_CHANNELS, 32, 32, PABULONG_SAMPLE_RATE, PABULONG_SAMPLE_RATE,
    },
    {   // 16-bit PCM, for clients that ask for it.
        {sizeof(KSDATARANGE_AUDIO), 0, 0, 0, STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO), STATICGUIDOF(KSDATAFORMAT_SUBTYPE_PCM),
         STATICGUIDOF(KSDATAFORMAT_SPECIFIER_WAVEFORMATEX)},
        PABULONG_CHANNELS, 16, 16, PABULONG_SAMPLE_RATE, PABULONG_SAMPLE_RATE,
    },
};

static PKSDATARANGE HostRangePointers[] = {PKSDATARANGE(&HostRanges[0]), PKSDATARANGE(&HostRanges[1])};

static KSDATARANGE BridgeRanges[] = {
    {sizeof(KSDATARANGE), 0, 0, 0, STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO), STATICGUIDOF(KSDATAFORMAT_SUBTYPE_ANALOG), STATICGUIDOF(KSDATAFORMAT_SPECIFIER_NONE)},
};

static PKSDATARANGE BridgeRangePointers[] = {&BridgeRanges[0]};

static PCPIN_DESCRIPTOR WavePins[] = {
    // PABULONG_WAVE_PIN_BRIDGE: from the topology filter.
    {0, 0, 0, NULL,
     {0, NULL, 0, NULL, SIZEOF_ARRAY(BridgeRangePointers), BridgeRangePointers, KSPIN_DATAFLOW_IN, KSPIN_COMMUNICATION_NONE, &KSCATEGORY_AUDIO, NULL, 0}},
    // PABULONG_WAVE_PIN_HOST: the capture stream the audio engine opens.
    {1, 1, 0, NULL,
     {0, NULL, 0, NULL, SIZEOF_ARRAY(HostRangePointers), HostRangePointers, KSPIN_DATAFLOW_OUT, KSPIN_COMMUNICATION_SINK, &KSCATEGORY_AUDIO,
      &KSAUDFNAME_RECORDING_CONTROL, 0}},
};

static PCNODE_DESCRIPTOR WaveNodes[] = {
    {0, NULL, &KSNODETYPE_ADC, NULL},
};

static PCCONNECTION_DESCRIPTOR WaveConnections[] = {
    {PCFILTER_NODE, PABULONG_WAVE_PIN_BRIDGE, 0, 1},  // bridge -> ADC input
    {0, 0, PCFILTER_NODE, PABULONG_WAVE_PIN_HOST},    // ADC output -> host pin
};

static PCFILTER_DESCRIPTOR WaveFilter = {
    0, NULL,
    sizeof(PCPIN_DESCRIPTOR), SIZEOF_ARRAY(WavePins), WavePins,
    sizeof(PCNODE_DESCRIPTOR), SIZEOF_ARRAY(WaveNodes), WaveNodes,
    SIZEOF_ARRAY(WaveConnections), WaveConnections,
    0, NULL,
};

// Returns TRUE for the formats in HostRanges; *isFloat says which.
static BOOLEAN ParseFormat(PKSDATAFORMAT format, BOOLEAN* isFloat)
{
    if (format == NULL || format->FormatSize < sizeof(KSDATAFORMAT) + sizeof(WAVEFORMATEX) ||
        !IsEqualGUIDAligned(format->MajorFormat, KSDATAFORMAT_TYPE_AUDIO) ||
        !IsEqualGUIDAligned(format->Specifier, KSDATAFORMAT_SPECIFIER_WAVEFORMATEX)) {
        return FALSE;
    }
    PWAVEFORMATEX wfx = (PWAVEFORMATEX)(format + 1);
    if (wfx->nChannels != PABULONG_CHANNELS || wfx->nSamplesPerSec != PABULONG_SAMPLE_RATE) {
        return FALSE;
    }
    if (IsEqualGUIDAligned(format->SubFormat, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT) && wfx->wBitsPerSample == 32) {
        *isFloat = TRUE;
        return TRUE;
    }
    if (IsEqualGUIDAligned(format->SubFormat, KSDATAFORMAT_SUBTYPE_PCM) && wfx->wBitsPerSample == 16) {
        *isFloat = FALSE;
        return TRUE;
    }
    return FALSE;
}

// -------------------------------------------------------------- stream

struct NotificationEntry {
    LIST_ENTRY Link;
    PKEVENT Event;
};

EXT_CALLBACK PabulongTimerCallback;

class CPabulongStream : public IMiniportWaveRTStreamNotification, public CUnknown {
public:
    DECLARE_STD_UNKNOWN();
    DEFINE_STD_CONSTRUCTOR(CPabulongStream);
    ~CPabulongStream();

    IMP_IMiniportWaveRTStream;
    IMP_IMiniportWaveRTStreamNotification;

    NTSTATUS Init(PPORTWAVERTSTREAM portStream, BOOLEAN isFloat);
    void OnTimer();

private:
    void UpdatePosition(LONGLONG qpc);           // lock held
    void SignalIfPacketDone();                   // lock held
    NTSTATUS AllocateBuffer(ULONG requested, ULONG notifications, PMDL* mdl, ULONG* actual, ULONG* offset, MEMORY_CACHING_TYPE* cache);
    void FreeBuffer(PMDL mdl);

    PPORTWAVERTSTREAM m_PortStream = NULL;
    BOOLEAN m_Float = TRUE;
    ULONG m_BlockAlign = 4;
    KSPIN_LOCK m_Lock;
    PEX_TIMER m_Timer = NULL;
    KSSTATE m_State = KSSTATE_STOP;
    BOOLEAN m_CountedRunning = FALSE;

    PUCHAR m_Buffer = NULL;
    ULONG m_BufferFrames = 0;
    ULONG m_FramesPerPacket = 0;

    LONGLONG m_QpcFrequency = 1;
    LONGLONG m_RunQpc = 0;
    ULONG64 m_RunFrames = 0;
    ULONG64 m_LinearFrames = 0;
    ULONG64 m_LastPacket = 0;

    float m_Frame[PABULONG_FRAME_LENGTH];
    ULONG m_FrameOffset = PABULONG_FRAME_LENGTH;
    LIST_ENTRY m_Events;
};

NTSTATUS CPabulongStream::Init(PPORTWAVERTSTREAM portStream, BOOLEAN isFloat)
{
    m_PortStream = portStream;
    m_PortStream->AddRef();
    m_Float = isFloat;
    m_BlockAlign = isFloat ? 4 : 2;
    KeInitializeSpinLock(&m_Lock);
    InitializeListHead(&m_Events);
    LARGE_INTEGER frequency;
    KeQueryPerformanceCounter(&frequency);
    m_QpcFrequency = frequency.QuadPart > 0 ? frequency.QuadPart : 1;
    m_Timer = ExAllocateTimer(PabulongTimerCallback, this, EX_TIMER_HIGH_RESOLUTION);
    return m_Timer != NULL ? STATUS_SUCCESS : STATUS_INSUFFICIENT_RESOURCES;
}

CPabulongStream::~CPabulongStream()
{
    if (m_Timer != NULL) {
        // Cancel and wait for any running callback before the object goes.
        ExDeleteTimer(m_Timer, TRUE, TRUE, NULL);
        m_Timer = NULL;
    }
    if (m_CountedRunning) {
        PabulongStreamRunning(FALSE);
        m_CountedRunning = FALSE;
    }
    while (!IsListEmpty(&m_Events)) {
        PLIST_ENTRY link = RemoveHeadList(&m_Events);
        ExFreePoolWithTag(CONTAINING_RECORD(link, NotificationEntry, Link), PABULONG_POOLTAG);
    }
    if (m_PortStream != NULL) {
        m_PortStream->Release();
        m_PortStream = NULL;
    }
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::NonDelegatingQueryInterface(_In_ REFIID iid, _COM_Outptr_ PVOID* object)
{
    if (IsEqualGUIDAligned(iid, IID_IUnknown)) {
        *object = PVOID(PUNKNOWN(PMINIPORTWAVERTSTREAM(this)));
    } else if (IsEqualGUIDAligned(iid, IID_IMiniportWaveRTStream)) {
        *object = PVOID(PMINIPORTWAVERTSTREAM(this));
    } else if (IsEqualGUIDAligned(iid, IID_IMiniportWaveRTStreamNotification)) {
        *object = PVOID(PMINIPORTWAVERTSTREAMNOTIFICATION(this));
    } else {
        *object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    PUNKNOWN(*object)->AddRef();
    return STATUS_SUCCESS;
}

// Frames captured between `m_RunQpc` and `qpc`, without 64-bit overflow.
static ULONG64 FramesSince(LONGLONG elapsed, LONGLONG frequency)
{
    if (elapsed <= 0) {
        return 0;
    }
    const ULONG64 e = (ULONG64)elapsed;
    const ULONG64 f = (ULONG64)frequency;
    return (e / f) * PABULONG_SAMPLE_RATE + ((e % f) * PABULONG_SAMPLE_RATE) / f;
}

void CPabulongStream::UpdatePosition(LONGLONG qpc)
{
    if (m_State != KSSTATE_RUN || m_Buffer == NULL || m_BufferFrames == 0) {
        return;
    }
    const ULONG64 target = m_RunFrames + FramesSince(qpc - m_RunQpc, m_QpcFrequency);
    if (target <= m_LinearFrames) {
        return;
    }
    // After a long stall, only the last buffer's worth can matter.
    if (target - m_LinearFrames > m_BufferFrames) {
        m_LinearFrames = target - m_BufferFrames;
    }
    PPABULONG_RING_HEADER ring = PabulongRingHeader();
    while (m_LinearFrames < target) {
        if (m_FrameOffset >= PABULONG_FRAME_LENGTH) {
            if (ring != NULL) {
                PabulongRingReadLatest(ring, m_Frame);
            } else {
                RtlZeroMemory(m_Frame, sizeof(m_Frame));
            }
            m_FrameOffset = 0;
        }
        const ULONG position = (ULONG)(m_LinearFrames % m_BufferFrames);
        ULONG64 count = target - m_LinearFrames;
        count = min(count, (ULONG64)(PABULONG_FRAME_LENGTH - m_FrameOffset));
        count = min(count, (ULONG64)(m_BufferFrames - position));
        const ULONG n = (ULONG)count;
        if (m_Float) {
            RtlCopyMemory(m_Buffer + (SIZE_T)position * 4, &m_Frame[m_FrameOffset], (SIZE_T)n * 4);
        } else {
            short* out = (short*)(m_Buffer + (SIZE_T)position * 2);
            for (ULONG i = 0; i < n; ++i) {
                out[i] = PabulongFloatToPcm16(m_Frame[m_FrameOffset + i]);
            }
        }
        m_FrameOffset += n;
        m_LinearFrames += n;
    }
}

void CPabulongStream::SignalIfPacketDone()
{
    if (m_FramesPerPacket == 0 || m_State != KSSTATE_RUN) {
        return;
    }
    const ULONG64 packet = m_LinearFrames / m_FramesPerPacket;
    if (packet == m_LastPacket) {
        return;
    }
    m_LastPacket = packet;
    for (PLIST_ENTRY link = m_Events.Flink; link != &m_Events; link = link->Flink) {
        KeSetEvent(CONTAINING_RECORD(link, NotificationEntry, Link)->Event, 0, FALSE);
    }
}

void CPabulongStream::OnTimer()
{
    KeAcquireSpinLockAtDpcLevel(&m_Lock);
    UpdatePosition(KeQueryPerformanceCounter(NULL).QuadPart);
    SignalIfPacketDone();
    KeReleaseSpinLockFromDpcLevel(&m_Lock);
}

void PabulongTimerCallback(PEX_TIMER, PVOID context)
{
    static_cast<CPabulongStream*>(context)->OnTimer();
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::SetFormat(_In_ PKSDATAFORMAT)
{
    // The format is fixed when the stream is created.
    return STATUS_NOT_SUPPORTED;
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::SetState(_In_ KSSTATE state)
{
    KIRQL irql;
    switch (state) {
    case KSSTATE_STOP:
        KeAcquireSpinLock(&m_Lock, &irql);
        m_State = KSSTATE_STOP;
        m_LinearFrames = 0;
        m_RunFrames = 0;
        m_LastPacket = 0;
        m_FrameOffset = PABULONG_FRAME_LENGTH;
        KeReleaseSpinLock(&m_Lock, irql);
        break;
    case KSSTATE_ACQUIRE:
        KeAcquireSpinLock(&m_Lock, &irql);
        m_State = KSSTATE_ACQUIRE;
        KeReleaseSpinLock(&m_Lock, irql);
        break;
    case KSSTATE_PAUSE:
        if (m_State == KSSTATE_RUN) {
            ExCancelTimer(m_Timer, NULL);
            KeAcquireSpinLock(&m_Lock, &irql);
            UpdatePosition(KeQueryPerformanceCounter(NULL).QuadPart);
            m_State = KSSTATE_PAUSE;
            KeReleaseSpinLock(&m_Lock, irql);
            if (m_CountedRunning) {
                PabulongStreamRunning(FALSE);
                m_CountedRunning = FALSE;
            }
        } else {
            KeAcquireSpinLock(&m_Lock, &irql);
            m_State = KSSTATE_PAUSE;
            KeReleaseSpinLock(&m_Lock, irql);
        }
        break;
    case KSSTATE_RUN:
        KeAcquireSpinLock(&m_Lock, &irql);
        m_RunQpc = KeQueryPerformanceCounter(NULL).QuadPart;
        m_RunFrames = m_LinearFrames;
        m_State = KSSTATE_RUN;
        KeReleaseSpinLock(&m_Lock, irql);
        // 1 ms period: fills the buffer and fires packet events on time.
        ExSetTimer(m_Timer, -10000, 10000, NULL);
        if (!m_CountedRunning) {
            // Tells the app a call app is reading Pabulong Mic.
            PabulongStreamRunning(TRUE);
            m_CountedRunning = TRUE;
        }
        break;
    default:
        return STATUS_INVALID_PARAMETER;
    }
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::GetPosition(_Out_ KSAUDIO_POSITION* position)
{
    KIRQL irql;
    KeAcquireSpinLock(&m_Lock, &irql);
    UpdatePosition(KeQueryPerformanceCounter(NULL).QuadPart);
    const ULONGLONG offset = m_BufferFrames ? (m_LinearFrames % m_BufferFrames) * m_BlockAlign : 0;
    KeReleaseSpinLock(&m_Lock, irql);
    position->PlayOffset = offset;
    position->WriteOffset = offset;
    return STATUS_SUCCESS;
}

NTSTATUS CPabulongStream::AllocateBuffer(ULONG requested, ULONG notifications, PMDL* mdlOut, ULONG* actual, ULONG* offset, MEMORY_CACHING_TYPE* cache)
{
    ULONG bytes = requested - requested % m_BlockAlign;
    if (bytes < m_BlockAlign * PABULONG_FRAME_LENGTH) {
        bytes = m_BlockAlign * PABULONG_FRAME_LENGTH;  // at least 10 ms
    }
    PHYSICAL_ADDRESS high;
    high.QuadPart = (LONGLONG)-1;
    PMDL mdl = m_PortStream->AllocatePagesForMdl(high, bytes);
    if (mdl == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    PUCHAR buffer = (PUCHAR)m_PortStream->MapAllocatedPages(mdl, MmCached);
    if (buffer == NULL) {
        m_PortStream->FreePagesFromMdl(mdl);
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    RtlZeroMemory(buffer, bytes);
    KIRQL irql;
    KeAcquireSpinLock(&m_Lock, &irql);
    m_Buffer = buffer;
    m_BufferFrames = bytes / m_BlockAlign;
    m_FramesPerPacket = notifications ? m_BufferFrames / notifications : 0;
    m_LastPacket = m_FramesPerPacket ? m_LinearFrames / m_FramesPerPacket : 0;
    KeReleaseSpinLock(&m_Lock, irql);
    *mdlOut = mdl;
    *actual = bytes;
    *offset = 0;
    *cache = MmCached;
    return STATUS_SUCCESS;
}

void CPabulongStream::FreeBuffer(PMDL mdl)
{
    KIRQL irql;
    KeAcquireSpinLock(&m_Lock, &irql);
    PUCHAR buffer = m_Buffer;
    m_Buffer = NULL;
    m_BufferFrames = 0;
    m_FramesPerPacket = 0;
    KeReleaseSpinLock(&m_Lock, irql);
    if (mdl != NULL) {
        if (buffer != NULL) {
            m_PortStream->UnmapAllocatedPages(buffer, mdl);
        }
        m_PortStream->FreePagesFromMdl(mdl);
    }
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::AllocateAudioBuffer(_In_ ULONG requestedSize, _Out_ PMDL* audioBufferMdl, _Out_ ULONG* actualSize,
                                                             _Out_ ULONG* offsetFromFirstPage, _Out_ MEMORY_CACHING_TYPE* cacheType)
{
    return AllocateBuffer(requestedSize, 0, audioBufferMdl, actualSize, offsetFromFirstPage, cacheType);
}

STDMETHODIMP_(VOID) CPabulongStream::FreeAudioBuffer(_In_opt_ PMDL mdl, _In_ ULONG)
{
    FreeBuffer(mdl);
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::AllocateBufferWithNotification(_In_ ULONG notificationCount, _In_ ULONG requestedSize,
                                                                        _Out_ PMDL* audioBufferMdl, _Out_ ULONG* actualSize,
                                                                        _Out_ ULONG* offsetFromFirstPage, _Out_ MEMORY_CACHING_TYPE* cacheType)
{
    if (notificationCount == 0) {
        return STATUS_INVALID_PARAMETER;
    }
    return AllocateBuffer(requestedSize, notificationCount, audioBufferMdl, actualSize, offsetFromFirstPage, cacheType);
}

STDMETHODIMP_(VOID) CPabulongStream::FreeBufferWithNotification(_In_ PMDL mdl, _In_ ULONG)
{
    FreeBuffer(mdl);
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::RegisterNotificationEvent(_In_ PKEVENT event)
{
    NotificationEntry* entry = (NotificationEntry*)ExAllocatePool2(POOL_FLAG_NON_PAGED, sizeof(NotificationEntry), PABULONG_POOLTAG);
    if (entry == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    entry->Event = event;
    KIRQL irql;
    KeAcquireSpinLock(&m_Lock, &irql);
    InsertTailList(&m_Events, &entry->Link);
    KeReleaseSpinLock(&m_Lock, irql);
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::UnregisterNotificationEvent(_In_ PKEVENT event)
{
    NotificationEntry* found = NULL;
    KIRQL irql;
    KeAcquireSpinLock(&m_Lock, &irql);
    for (PLIST_ENTRY link = m_Events.Flink; link != &m_Events; link = link->Flink) {
        NotificationEntry* entry = CONTAINING_RECORD(link, NotificationEntry, Link);
        if (entry->Event == event) {
            RemoveEntryList(link);
            found = entry;
            break;
        }
    }
    KeReleaseSpinLock(&m_Lock, irql);
    if (found == NULL) {
        return STATUS_NOT_FOUND;
    }
    ExFreePoolWithTag(found, PABULONG_POOLTAG);
    return STATUS_SUCCESS;
}

STDMETHODIMP_(VOID) CPabulongStream::GetHWLatency(_Out_ KSRTAUDIO_HWLATENCY* latency)
{
    latency->ChipsetDelay = 0;
    latency->CodecDelay = 0;
    latency->FifoSize = 0;
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::GetPositionRegister(_Out_ KSRTAUDIO_HWREGISTER*)
{
    return STATUS_NOT_IMPLEMENTED;
}

STDMETHODIMP_(NTSTATUS) CPabulongStream::GetClockRegister(_Out_ KSRTAUDIO_HWREGISTER*)
{
    return STATUS_NOT_IMPLEMENTED;
}

// ------------------------------------------------------------ miniport

class CPabulongWave : public IMiniportWaveRT, public CUnknown {
public:
    DECLARE_STD_UNKNOWN();
    DEFINE_STD_CONSTRUCTOR(CPabulongWave);
    ~CPabulongWave() {}
    IMP_IMiniportWaveRT;
};

STDMETHODIMP_(NTSTATUS) CPabulongWave::NonDelegatingQueryInterface(_In_ REFIID iid, _COM_Outptr_ PVOID* object)
{
    if (IsEqualGUIDAligned(iid, IID_IUnknown)) {
        *object = PVOID(PUNKNOWN(PMINIPORTWAVERT(this)));
    } else if (IsEqualGUIDAligned(iid, IID_IMiniport)) {
        *object = PVOID(PMINIPORT(this));
    } else if (IsEqualGUIDAligned(iid, IID_IMiniportWaveRT)) {
        *object = PVOID(PMINIPORTWAVERT(this));
    } else {
        *object = NULL;
        return STATUS_INVALID_PARAMETER;
    }
    PUNKNOWN(*object)->AddRef();
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongWave::Init(_In_ PUNKNOWN, _In_ PRESOURCELIST, _In_ PPORTWAVERT)
{
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongWave::GetDescription(_Out_ PPCFILTER_DESCRIPTOR* description)
{
    *description = &WaveFilter;
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CPabulongWave::DataRangeIntersection(_In_ ULONG PinId, _In_ PKSDATARANGE DataRange, _In_ PKSDATARANGE MatchingDataRange,
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

STDMETHODIMP_(NTSTATUS) CPabulongWave::NewStream(_Out_ PMINIPORTWAVERTSTREAM* stream, _In_ PPORTWAVERTSTREAM portStream, _In_ ULONG pin,
                                                 _In_ BOOLEAN capture, _In_ PKSDATAFORMAT format)
{
    *stream = NULL;
    BOOLEAN isFloat = TRUE;
    if (pin != PABULONG_WAVE_PIN_HOST || !capture || !ParseFormat(format, &isFloat)) {
        return STATUS_INVALID_PARAMETER;
    }
    CPabulongStream* s = new (POOL_FLAG_NON_PAGED, PABULONG_POOLTAG) CPabulongStream(NULL);
    if (s == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    s->AddRef();
    NTSTATUS status = s->Init(portStream, isFloat);
    if (NT_SUCCESS(status)) {
        *stream = PMINIPORTWAVERTSTREAM(s);
        (*stream)->AddRef();
    }
    s->Release();
    return status;
}

STDMETHODIMP_(NTSTATUS) CPabulongWave::GetDeviceDescription(_Out_ PDEVICE_DESCRIPTION description)
{
    RtlZeroMemory(description, sizeof(DEVICE_DESCRIPTION));
    description->Master = TRUE;
    description->ScatterGather = TRUE;
    description->Dma32BitAddresses = TRUE;
    description->InterfaceType = PCIBus;
    description->MaximumLength = 0xFFFFFFFF;
    return STATUS_SUCCESS;
}

NTSTATUS CreateWaveMiniport(PUNKNOWN* unknown)
{
    CPabulongWave* wave = new (POOL_FLAG_NON_PAGED, PABULONG_POOLTAG) CPabulongWave(NULL);
    if (wave == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    *unknown = PUNKNOWN(PMINIPORTWAVERT(wave));
    (*unknown)->AddRef();
    return STATUS_SUCCESS;
}
