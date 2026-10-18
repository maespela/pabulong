// Pabulong Mic: a virtual capture device ("Pabulong Mic") whose audio comes
// from the Pabulong app through a shared-memory ring.
//
// Written from the Windows Driver Kit's PortCls/WaveRT documentation; it is
// not a copy of the SysVAD sample.
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#pragma once

#include <portcls.h>
#include <stdunk.h>
#include <ksdebug.h>
#include <ntintsafe.h>
#include <wdmsec.h>

#include "pabulong_ring.h"

#define PABULONG_POOLTAG 'MbaP'

#define PABULONG_WAVE_NAME      L"WavePabulongMic"
#define PABULONG_TOPOLOGY_NAME  L"TopologyPabulongMic"

// Pins of the wave filter.
#define PABULONG_WAVE_PIN_BRIDGE 0   // in, from topology
#define PABULONG_WAVE_PIN_HOST   1   // out, to the audio engine
// Pins of the topology filter.
#define PABULONG_TOPO_PIN_MIC    0   // in, the "microphone"
#define PABULONG_TOPO_PIN_BRIDGE 1   // out, to the wave filter

// Kernel new/delete with POOL_FLAGS (stdunk's are disabled by
// _NEW_DELETE_OPERATORS_ in the project).
PVOID operator new(size_t size, POOL_FLAGS flags, ULONG tag);
void __cdecl operator delete(PVOID p);
void __cdecl operator delete(PVOID p, size_t size);
void __cdecl operator delete(PVOID p, ULONG tag);
void __cdecl operator delete[](PVOID p);

// ring.cpp — the shared ring and the control device \\.\PabulongMic.
NTSTATUS PabulongRingAllocate();
void PabulongRingFree();
PPABULONG_RING_HEADER PabulongRingHeader();      // kernel mapping, always valid
void PabulongStreamRunning(BOOLEAN running);     // consumer_running_count
NTSTATUS PabulongControlCreate(PDRIVER_OBJECT driver);
void PabulongControlDelete();
void PabulongControlHookDispatch(PDRIVER_OBJECT driver);

// wave.cpp / topology.cpp
NTSTATUS CreateWaveMiniport(PUNKNOWN* unknown);
NTSTATUS CreateTopologyMiniport(PUNKNOWN* unknown);
