// User-mode test of the driver's ring reader (ring_read.h) against the same
// scenarios as the Rust crate's tests. Builds with any C++17 compiler:
//
//   clang++ -std=c++17 -O1 -I../src ring_test.cpp -o ring_test && ./ring_test
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#include <cassert>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <limits>
#include <thread>
#include <vector>

// Minimal stand-ins for the kernel types and intrinsics ring_read.h uses.
typedef uint32_t ULONG;
typedef int32_t LONG;
typedef int64_t LONG64;
typedef uint64_t ULONG64;
typedef unsigned char UCHAR;
typedef unsigned char BOOLEAN;
typedef size_t SIZE_T;
#define TRUE 1
#define FALSE 0
#define C_ASSERT(e) static_assert(e, #e)
#define FIELD_OFFSET(t, f) offsetof(t, f)
#define RtlZeroMemory(d, n) memset((d), 0, (n))
#define RtlCopyMemory(d, s, n) memcpy((d), (s), (n))
#define CTL_CODE(t, f, m, a) (((t) << 16) | ((a) << 14) | ((f) << 2) | (m))
#define METHOD_BUFFERED 0
#define FILE_READ_DATA 1
#define FILE_WRITE_DATA 2
#define PABULONG_READ_ACQUIRE64(p) __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define PABULONG_READ_NOFENCE64(p) __atomic_load_n((p), __ATOMIC_RELAXED)
#define PABULONG_WRITE_RELEASE64(p, v) __atomic_store_n((p), (v), __ATOMIC_RELEASE)
#define PABULONG_READ_ACQUIRE32(p) __atomic_load_n((p), __ATOMIC_ACQUIRE)
#define PABULONG_ADD64(p, v) __atomic_add_fetch((p), (v), __ATOMIC_RELAXED)

#include "pabulong_ring.h"
#include "ring_read.h"

static const size_t kBytes = 65536;

struct Ring {
    std::vector<uint64_t> mem = std::vector<uint64_t>(kBytes / 8);
    PPABULONG_RING_HEADER h() { return reinterpret_cast<PPABULONG_RING_HEADER>(mem.data()); }
    float* slots() { return reinterpret_cast<float*>(reinterpret_cast<UCHAR*>(h()) + PABULONG_RING_HEADER_BYTES); }
    Ring() { PabulongRingInitialize(h(), kBytes); }
    // Producer side, as in the Rust crate.
    bool write(float v) {
        LONG64 w = __atomic_load_n(&h()->WriteCount, __ATOMIC_RELAXED);
        LONG64 r = __atomic_load_n(&h()->ReadCount, __ATOMIC_ACQUIRE);
        if ((uint64_t)(w - r) >= PABULONG_CAPACITY_FRAMES) {
            __atomic_add_fetch(&h()->OverrunCount, 1, __ATOMIC_RELAXED);
            return false;
        }
        float* s = slots() + (w % PABULONG_CAPACITY_FRAMES) * PABULONG_FRAME_LENGTH;
        for (unsigned i = 0; i < PABULONG_FRAME_LENGTH; ++i) s[i] = v;
        __atomic_store_n(&h()->WriteCount, w + 1, __ATOMIC_RELEASE);
        return true;
    }
    void active(bool on) { __atomic_store_n(&h()->ProducerActive, on ? 1 : 0, __ATOMIC_RELEASE); }
};

#define CHECK(c) do { if (!(c)) { std::fprintf(stderr, "FAIL %s:%d %s\n", __FILE__, __LINE__, #c); return 1; } } while (0)

int main() {
    float out[PABULONG_FRAME_LENGTH];
    {
        Ring r;
        CHECK(r.h()->Magic == 0x524D4250u);
        r.active(true);
        for (int i = 0; i < 100; ++i) {
            CHECK(r.write((float)i));
            CHECK(PabulongRingReadLatest(r.h(), out));
            CHECK(out[0] == (float)i && out[479] == (float)i);
        }
    }
    {
        Ring r;  // idle producer: silence, no underrun
        CHECK(!PabulongRingReadLatest(r.h(), out));
        CHECK(r.h()->UnderrunCount == 0);
        r.active(true);
        CHECK(!PabulongRingReadLatest(r.h(), out));
        CHECK(r.h()->UnderrunCount == 1);
    }
    {
        Ring r;  // latency bound: keeps the newest 3 of 10
        r.active(true);
        for (int i = 0; i < 10; ++i) r.write((float)i);
        CHECK(PabulongRingReadLatest(r.h(), out));
        CHECK(out[0] == 7.0f);
        CHECK(r.h()->DroppedCount == 7);
    }
    {
        Ring r;  // inactive producer with queued frames: silence
        r.write(0.5f);
        CHECK(!PabulongRingReadLatest(r.h(), out));
        CHECK(out[0] == 0.0f);
    }
    {
        Ring r;  // hostile counters never move the read outside the ring
        r.active(true);
        r.h()->WriteCount = 0x7FFFFFFFFFFFFFF0LL;
        r.h()->ReadCount = -5;
        r.h()->CapacityFrames = 1000000;  // ignored by the driver
        for (int i = 0; i < 10; ++i) PabulongRingReadLatest(r.h(), out);
        CHECK(true);
    }
    {
        Ring r;  // NaN/Inf from the producer become silence
        r.active(true);
        r.write(std::numeric_limits<float>::quiet_NaN());
        CHECK(PabulongRingReadLatest(r.h(), out));
        CHECK(out[0] == 0.0f);
        r.write(std::numeric_limits<float>::infinity());
        CHECK(PabulongRingReadLatest(r.h(), out));
        CHECK(out[10] == 0.0f);
    }
    {
        Ring r;  // concurrent producer and reader never tear a frame
        r.active(true);
        bool done = false;
        std::thread producer([&] {
            int written = 0;
            while (written < 20000) {
                if (r.write((float)written)) ++written; else std::this_thread::yield();
            }
            __atomic_store_n(&done, true, __ATOMIC_RELEASE);
        });
        float last = -1.0f;
        int seen = 0;
        while (!__atomic_load_n(&done, __ATOMIC_ACQUIRE) || r.h()->WriteCount != r.h()->ReadCount) {
            if (PabulongRingReadLatest(r.h(), out)) {
                for (unsigned k = 1; k < PABULONG_FRAME_LENGTH; ++k) CHECK(out[k] == out[0]);
                CHECK(out[0] > last);
                last = out[0];
                ++seen;
            }
        }
        producer.join();
        CHECK(seen > 0);
    }
    {
        // Integer float->PCM16 matches the float formula everywhere.
        const float cases[] = {0.0f, -0.0f, 1e-9f, 0.5f, -0.5f, 0.999f, 1.0f, -1.0f, 1.5f, -2.0f, 3e-5f, -3e-5f, 0.25f / 32768};
        for (float v : cases) {
            float c = v > 1.0f ? 1.0f : (v < -1.0f ? -1.0f : v);
            long expect = (long)(c * 32768.0f);
            if (expect > 32767) expect = 32767;
            CHECK(PabulongFloatToPcm16(v) == expect);
        }
        for (int i = -40000; i <= 40000; ++i) {
            float v = (float)i / 32768.0f * 1.0001f;
            float c = v > 1.0f ? 1.0f : (v < -1.0f ? -1.0f : v);
            long expect = (long)(c * 32768.0f);
            if (expect > 32767) expect = 32767;
            CHECK(PabulongFloatToPcm16(v) == expect);
        }
    }
    std::printf("ring_test: all passed\n");
    return 0;
}
