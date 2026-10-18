// The driver's side of the shared ring: read the next 10 ms frame.
// Line-for-line port of Ring::read_frame_latest in
// windows/crates/pabulong-ring/src/lib.rs, kept in a header with no kernel
// dependencies so windows/driver/test/ring_test.cpp can test it in user mode.
//
// Security: the header lives in memory the producer process can write, so
// the geometry used here (capacity, frame length, latency bound) comes from
// compile-time constants, never from the shared header. Any counter values
// the producer writes can only change what this stream plays, never where
// the driver reads or writes.
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#pragma once

#ifndef PABULONG_READ_ACQUIRE64
#define PABULONG_READ_ACQUIRE64(p)      ReadAcquire64(p)
#define PABULONG_READ_NOFENCE64(p)      ReadNoFence64(p)
#define PABULONG_WRITE_RELEASE64(p, v)  WriteRelease64((p), (v))
#define PABULONG_READ_ACQUIRE32(p)      ReadAcquire(p)
#define PABULONG_ADD64(p, v)            InterlockedAdd64((p), (v))
#endif

// Replaces NaN/Inf with silence using integer tests only (no FPU state
// needed at DISPATCH_LEVEL).
static __inline void PabulongSanitize(float* samples, ULONG count)
{
    for (ULONG i = 0; i < count; ++i) {
        ULONG bits;
        RtlCopyMemory(&bits, &samples[i], sizeof(bits));
        if ((bits & 0x7F800000u) == 0x7F800000u) {
            samples[i] = 0.0f;
        }
    }
}

// float (-1..1) -> 16-bit PCM, x * 32768 truncated toward zero and clamped,
// computed from the IEEE-754 bits with integer arithmetic only, so the
// WaveRT timer callback never touches floating-point state.
static __inline short PabulongFloatToPcm16(float value)
{
    ULONG bits;
    RtlCopyMemory(&bits, &value, sizeof(bits));
    const ULONG exponent = (bits >> 23) & 0xFF;
    const ULONG mantissa = (bits & 0x7FFFFF) | 0x800000;  // implicit leading 1
    const BOOLEAN negative = (bits >> 31) != 0;
    // value * 2^15 = mantissa * 2^(exponent - 127 - 23 + 15)
    const LONG shift = (LONG)exponent - 135;
    LONG magnitude;
    if (exponent == 0 || shift < -24) {
        magnitude = 0;                       // zero, denormal or < 2^-16
    } else if (shift >= 0) {
        magnitude = 0x10000;                 // |value| >= 1: clamp below
    } else {
        magnitude = (LONG)(mantissa >> (ULONG)(-shift));
    }
    if (negative) {
        return (short)(magnitude > 32768 ? -32768 : -magnitude);
    }
    return (short)(magnitude > 32767 ? 32767 : magnitude);
}

// Fills out[PABULONG_FRAME_LENGTH] with the next frame, or silence.
// Returns TRUE when real audio was delivered.
static __inline BOOLEAN PabulongRingReadLatest(PPABULONG_RING_HEADER h, float* out)
{
    const ULONG64 capacity = PABULONG_CAPACITY_FRAMES;
    const ULONG64 maxBuffered = PABULONG_MAX_BUFFERED_FRAMES;
    const float* samples = (const float*)((UCHAR*)h + PABULONG_RING_HEADER_BYTES);

    LONG64 write = PABULONG_READ_ACQUIRE64(&h->WriteCount);
    LONG64 read = PABULONG_READ_NOFENCE64(&h->ReadCount);
    ULONG64 available = (ULONG64)(write - read);
    if (available > capacity) {
        // Counters out of range (e.g. a new producer): resynchronise.
        read = write;
        available = 0;
    }
    if (available > maxBuffered) {
        PABULONG_ADD64(&h->DroppedCount, (LONG64)(available - maxBuffered));
        read = write - (LONG64)maxBuffered;
        available = maxBuffered;
    }
    LONG active = PABULONG_READ_ACQUIRE32(&h->ProducerActive);
    if (available == 0 || active == 0) {
        RtlZeroMemory(out, PABULONG_FRAME_LENGTH * sizeof(float));
        if (available == 0 && active != 0) {
            PABULONG_ADD64(&h->UnderrunCount, 1);
        }
        PABULONG_WRITE_RELEASE64(&h->ReadCount, read);
        return FALSE;
    }
    ULONG64 slot = ((ULONG64)read) % capacity;
    RtlCopyMemory(out, samples + slot * PABULONG_FRAME_LENGTH, PABULONG_FRAME_LENGTH * sizeof(float));
    PabulongSanitize(out, PABULONG_FRAME_LENGTH);
    PABULONG_WRITE_RELEASE64(&h->ReadCount, read + 1);
    return TRUE;
}

// Writes a fresh header (the driver does this once, before any mapping).
static __inline void PabulongRingInitialize(PPABULONG_RING_HEADER h, SIZE_T bytes)
{
    RtlZeroMemory(h, bytes);
    h->Version = PABULONG_RING_VERSION;
    h->HeaderBytes = PABULONG_RING_HEADER_BYTES;
    h->SampleRate = PABULONG_SAMPLE_RATE;
    h->Channels = PABULONG_CHANNELS;
    h->FrameLength = PABULONG_FRAME_LENGTH;
    h->CapacityFrames = PABULONG_CAPACITY_FRAMES;
    h->MaxBufferedFrames = PABULONG_MAX_BUFFERED_FRAMES;
    h->DriverVersion = PABULONG_DRIVER_VERSION;
    h->Magic = PABULONG_RING_MAGIC;
}
