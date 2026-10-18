// Pabulong Mic shared ring: C mirror of windows/crates/pabulong-ring.
// Layout, field ownership and IOCTLs are specified in windows/driver/CONTRACT.md.
// Any change here must be made in the Rust crate too (its tests pin the
// same offsets and IOCTL values).
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#pragma once

#define PABULONG_RING_MAGIC            0x524D4250u   // "PBMR" little-endian
#define PABULONG_RING_VERSION          1u
#define PABULONG_RING_HEADER_BYTES     128u
#define PABULONG_SAMPLE_RATE           48000u
#define PABULONG_CHANNELS              1u
#define PABULONG_FRAME_LENGTH          480u          // 10 ms
#define PABULONG_CAPACITY_FRAMES       32u
#define PABULONG_MAX_BUFFERED_FRAMES   3u            // 30 ms, as on macOS
#define PABULONG_DRIVER_VERSION        1u

typedef struct _PABULONG_RING_HEADER {
    // Geometry: written once by the driver before the first mapping.
    ULONG Magic;                  //  0
    ULONG Version;                //  4
    ULONG HeaderBytes;            //  8
    ULONG SampleRate;             // 12
    ULONG Channels;               // 16
    ULONG FrameLength;            // 20
    ULONG CapacityFrames;         // 24
    ULONG MaxBufferedFrames;      // 28
    // Monotonic frame counters; slot = count % CapacityFrames.
    volatile LONG64 WriteCount;   // 32  producer
    volatile LONG64 ReadCount;    // 40  driver
    volatile LONG64 UnderrunCount;// 48  driver
    volatile LONG64 DroppedCount; // 56  driver
    volatile LONG64 OverrunCount; // 64  producer
    volatile LONG ProducerAttached;     // 72  driver
    volatile LONG ConsumerRunningCount; // 76  driver
    volatile LONG ProducerActive;       // 80  producer (driver clears on detach)
    ULONG DriverVersion;                // 84  driver
    UCHAR Reserved[40];                 // 88..128
    // float Samples[CapacityFrames * FrameLength] follows.
} PABULONG_RING_HEADER, *PPABULONG_RING_HEADER;

C_ASSERT(sizeof(PABULONG_RING_HEADER) == PABULONG_RING_HEADER_BYTES);
C_ASSERT(FIELD_OFFSET(PABULONG_RING_HEADER, WriteCount) == 32);
C_ASSERT(FIELD_OFFSET(PABULONG_RING_HEADER, OverrunCount) == 64);
C_ASSERT(FIELD_OFFSET(PABULONG_RING_HEADER, ProducerAttached) == 72);
C_ASSERT(FIELD_OFFSET(PABULONG_RING_HEADER, ConsumerRunningCount) == 76);
C_ASSERT(FIELD_OFFSET(PABULONG_RING_HEADER, ProducerActive) == 80);
C_ASSERT(FIELD_OFFSET(PABULONG_RING_HEADER, DriverVersion) == 84);

#define PABULONG_RING_BYTES(capacity, frame) \
    (PABULONG_RING_HEADER_BYTES + (SIZE_T)(capacity) * (frame) * sizeof(float))

// ---------------------------------------------------------------- IOCTLs

#define PABULONG_DEVICE_TYPE 0x9A5C

// Maps the ring into the calling process. Output: PABULONG_MAP_RING_OUTPUT.
// STATUS_DEVICE_BUSY if another handle holds the mapping (single producer).
#define IOCTL_PABULONG_MAP_RING \
    CTL_CODE(PABULONG_DEVICE_TYPE, 0x801, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA)
// Unmaps it (also done automatically when the handle is closed).
#define IOCTL_PABULONG_UNMAP_RING \
    CTL_CODE(PABULONG_DEVICE_TYPE, 0x802, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA)
// Output: PABULONG_DRIVER_INFO. Needs no mapping.
#define IOCTL_PABULONG_GET_INFO \
    CTL_CODE(PABULONG_DEVICE_TYPE, 0x803, METHOD_BUFFERED, FILE_READ_DATA)

C_ASSERT(IOCTL_PABULONG_MAP_RING == 0x9A5CE004);
C_ASSERT(IOCTL_PABULONG_UNMAP_RING == 0x9A5CE008);
C_ASSERT(IOCTL_PABULONG_GET_INFO == 0x9A5C600C);

typedef struct _PABULONG_MAP_RING_OUTPUT {
    ULONG64 UserAddress;
    ULONG Bytes;
    ULONG Version;
} PABULONG_MAP_RING_OUTPUT;
C_ASSERT(sizeof(PABULONG_MAP_RING_OUTPUT) == 16);

typedef struct _PABULONG_DRIVER_INFO {
    ULONG Version;
    ULONG DriverVersion;
    ULONG ConsumerRunningCount;
    ULONG ProducerAttached;
} PABULONG_DRIVER_INFO;
C_ASSERT(sizeof(PABULONG_DRIVER_INFO) == 16);

#define PABULONG_CONTROL_DEVICE_NAME  L"\\Device\\PabulongMic"
#define PABULONG_CONTROL_SYMLINK      L"\\DosDevices\\PabulongMic"   // user mode: \\.\PabulongMic
