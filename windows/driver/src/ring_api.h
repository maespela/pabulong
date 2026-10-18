// The shared ring and the control device (ring.cpp). Kept free of PortCls
// so ring.cpp can include <ntifs.h> (KeStackAttachProcess) instead.
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#pragma once

#include "pabulong_ring.h"

NTSTATUS PabulongRingAllocate();
void PabulongRingFree();
PPABULONG_RING_HEADER PabulongRingHeader();      // kernel mapping, always valid
void PabulongStreamRunning(BOOLEAN running);     // consumer_running_count
NTSTATUS PabulongControlCreate(PDRIVER_OBJECT driver);
void PabulongControlDelete();
void PabulongControlHookDispatch(PDRIVER_OBJECT driver);
