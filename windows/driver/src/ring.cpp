// The shared ring and the control device \\.\PabulongMic.
//
// The driver owns the ring: whole non-paged pages, mapped once into system
// space (read by the WaveRT timer at DISPATCH_LEVEL) and, on request, into
// the one process that holds the mapping. That process is the producer; a
// second one gets STATUS_DEVICE_BUSY, which is how the app knows another
// copy is already feeding Pabulong Mic (ProducerGuard on macOS).
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#include <ntifs.h>    // KeStackAttachProcess; this file needs no PortCls
#include <wdmsec.h>
#include "ring_api.h"
#include "ring_read.h"

namespace {

// {6C1B7E58-2B0F-4F38-9C1D-3A8E5C0B7F21}: class GUID for IoCreateDeviceSecure.
const GUID PabulongControlClass = {0x6c1b7e58, 0x2b0f, 0x4f38, {0x9c, 0x1d, 0x3a, 0x8e, 0x5c, 0x0b, 0x7f, 0x21}};

struct RingState {
    PMDL Mdl;
    PPABULONG_RING_HEADER Kernel;
    SIZE_T Bytes;
    FAST_MUTEX Mutex;          // serialises map/unmap (PASSIVE/APC_LEVEL)
    PFILE_OBJECT Owner;        // handle that holds the user mapping
    PVOID UserVa;
    PEPROCESS OwnerProcess;
    volatile LONG Running;     // capture streams in KSSTATE_RUN
    PDEVICE_OBJECT ControlDevice;
    BOOLEAN SymlinkCreated;
    PDRIVER_DISPATCH Original[IRP_MJ_MAXIMUM_FUNCTION + 1];
};

RingState g;

NTSTATUS MapRing(PFILE_OBJECT file, PABULONG_MAP_RING_OUTPUT* out)
{
    PAGED_CODE();
    NTSTATUS status = STATUS_SUCCESS;
    ExAcquireFastMutex(&g.Mutex);
    if (g.Owner == file) {
        // Already mapped for this handle.
    } else if (g.Owner != NULL) {
        status = STATUS_DEVICE_BUSY;
    } else {
        PVOID va = NULL;
        __try {
            va = MmMapLockedPagesSpecifyCache(g.Mdl, UserMode, MmCached, NULL, FALSE, NormalPagePriority | MdlMappingNoExecute);
        } __except (EXCEPTION_EXECUTE_HANDLER) {
            va = NULL;
        }
        if (va == NULL) {
            status = STATUS_INSUFFICIENT_RESOURCES;
        } else {
            g.Owner = file;
            g.UserVa = va;
            g.OwnerProcess = PsGetCurrentProcess();
            ObReferenceObject(g.OwnerProcess);
            InterlockedExchange(&g.Kernel->ProducerAttached, 1);
        }
    }
    if (NT_SUCCESS(status)) {
        out->UserAddress = (ULONG64)(ULONG_PTR)g.UserVa;
        out->Bytes = (ULONG)g.Bytes;
        out->Version = PABULONG_RING_VERSION;
    }
    ExReleaseFastMutex(&g.Mutex);
    return status;
}

void UnmapRing(PFILE_OBJECT file)
{
    PAGED_CODE();
    ExAcquireFastMutex(&g.Mutex);
    if (g.Owner != NULL && g.Owner == file) {
        KAPC_STATE apc;
        BOOLEAN attached = FALSE;
        // Cleanup normally runs in the owner's context; attach if not
        // (e.g. the handle was duplicated into another process).
        if (PsGetCurrentProcess() != g.OwnerProcess) {
            KeStackAttachProcess(g.OwnerProcess, &apc);
            attached = TRUE;
        }
        MmUnmapLockedPages(g.UserVa, g.Mdl);
        if (attached) {
            KeUnstackDetachProcess(&apc);
        }
        ObDereferenceObject(g.OwnerProcess);
        g.Owner = NULL;
        g.UserVa = NULL;
        g.OwnerProcess = NULL;
        // The producer is gone: Pabulong Mic plays silence until the next one.
        InterlockedExchange(&g.Kernel->ProducerActive, 0);
        InterlockedExchange(&g.Kernel->ProducerAttached, 0);
    }
    ExReleaseFastMutex(&g.Mutex);
}

NTSTATUS Complete(PIRP irp, NTSTATUS status, ULONG_PTR information)
{
    irp->IoStatus.Status = status;
    irp->IoStatus.Information = information;
    IoCompleteRequest(irp, IO_NO_INCREMENT);
    return status;
}

NTSTATUS ControlDispatch(PIRP irp)
{
    PAGED_CODE();
    PIO_STACK_LOCATION stack = IoGetCurrentIrpStackLocation(irp);
    switch (stack->MajorFunction) {
    case IRP_MJ_CREATE:
    case IRP_MJ_CLOSE:
        return Complete(irp, STATUS_SUCCESS, 0);
    case IRP_MJ_CLEANUP:
        UnmapRing(stack->FileObject);
        return Complete(irp, STATUS_SUCCESS, 0);
    case IRP_MJ_DEVICE_CONTROL:
        break;
    default:
        return Complete(irp, STATUS_INVALID_DEVICE_REQUEST, 0);
    }

    const ULONG code = stack->Parameters.DeviceIoControl.IoControlCode;
    const ULONG outLength = stack->Parameters.DeviceIoControl.OutputBufferLength;
    PVOID buffer = irp->AssociatedIrp.SystemBuffer;
    switch (code) {
    case IOCTL_PABULONG_MAP_RING: {
        if (irp->RequestorMode != UserMode) {
            return Complete(irp, STATUS_INVALID_DEVICE_REQUEST, 0);
        }
        if (buffer == NULL || outLength < sizeof(PABULONG_MAP_RING_OUTPUT)) {
            return Complete(irp, STATUS_BUFFER_TOO_SMALL, 0);
        }
        PABULONG_MAP_RING_OUTPUT out = {};
        NTSTATUS status = MapRing(stack->FileObject, &out);
        if (!NT_SUCCESS(status)) {
            return Complete(irp, status, 0);
        }
        RtlCopyMemory(buffer, &out, sizeof(out));
        return Complete(irp, STATUS_SUCCESS, sizeof(out));
    }
    case IOCTL_PABULONG_UNMAP_RING:
        UnmapRing(stack->FileObject);
        return Complete(irp, STATUS_SUCCESS, 0);
    case IOCTL_PABULONG_GET_INFO: {
        if (buffer == NULL || outLength < sizeof(PABULONG_DRIVER_INFO)) {
            return Complete(irp, STATUS_BUFFER_TOO_SMALL, 0);
        }
        PABULONG_DRIVER_INFO info = {};
        info.Version = PABULONG_RING_VERSION;
        info.DriverVersion = PABULONG_DRIVER_VERSION;
        info.ConsumerRunningCount = (ULONG)ReadAcquire(&g.Kernel->ConsumerRunningCount);
        info.ProducerAttached = (ULONG)ReadAcquire(&g.Kernel->ProducerAttached);
        RtlCopyMemory(buffer, &info, sizeof(info));
        return Complete(irp, STATUS_SUCCESS, sizeof(info));
    }
    default:
        return Complete(irp, STATUS_INVALID_DEVICE_REQUEST, 0);
    }
}

_Dispatch_type_(IRP_MJ_CREATE)
_Dispatch_type_(IRP_MJ_CLOSE)
_Dispatch_type_(IRP_MJ_CLEANUP)
_Dispatch_type_(IRP_MJ_DEVICE_CONTROL)
DRIVER_DISPATCH HookedDispatch;

NTSTATUS HookedDispatch(PDEVICE_OBJECT device, PIRP irp)
{
    if (device != NULL && device == g.ControlDevice) {
        return ControlDispatch(irp);
    }
    // Everything else belongs to PortCls.
    const UCHAR major = IoGetCurrentIrpStackLocation(irp)->MajorFunction;
    return g.Original[major](device, irp);
}

} // namespace

#pragma code_seg("INIT")
NTSTATUS PabulongRingAllocate()
{
    ExInitializeFastMutex(&g.Mutex);
    g.Bytes = ROUND_TO_PAGES(PABULONG_RING_BYTES(PABULONG_CAPACITY_FRAMES, PABULONG_FRAME_LENGTH));
    PHYSICAL_ADDRESS low = {};
    PHYSICAL_ADDRESS high;
    high.QuadPart = (LONGLONG)-1;
    PHYSICAL_ADDRESS skip = {};
    g.Mdl = MmAllocatePagesForMdlEx(low, high, skip, g.Bytes, MmCached, MM_ALLOCATE_FULLY_REQUIRED);
    if (g.Mdl == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    g.Kernel = (PPABULONG_RING_HEADER)MmGetSystemAddressForMdlSafe(g.Mdl, NormalPagePriority | MdlMappingNoExecute);
    if (g.Kernel == NULL) {
        MmFreePagesFromMdl(g.Mdl);
        ExFreePool(g.Mdl);
        g.Mdl = NULL;
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    PabulongRingInitialize(g.Kernel, g.Bytes);
    return STATUS_SUCCESS;
}
#pragma code_seg()

#pragma code_seg("PAGE")
void PabulongRingFree()
{
    PAGED_CODE();
    if (g.Mdl != NULL) {
        if (g.Kernel != NULL) {
            MmUnmapLockedPages(g.Kernel, g.Mdl);
            g.Kernel = NULL;
        }
        MmFreePagesFromMdl(g.Mdl);
        ExFreePool(g.Mdl);
        g.Mdl = NULL;
    }
}

void PabulongControlHookDispatch(PDRIVER_OBJECT driver)
{
    PAGED_CODE();
    const UCHAR majors[] = {IRP_MJ_CREATE, IRP_MJ_CLOSE, IRP_MJ_CLEANUP, IRP_MJ_DEVICE_CONTROL};
    for (UCHAR major : majors) {
        g.Original[major] = driver->MajorFunction[major];
        driver->MajorFunction[major] = HookedDispatch;
    }
}

NTSTATUS PabulongControlCreate(PDRIVER_OBJECT driver)
{
    PAGED_CODE();
    if (g.ControlDevice != NULL) {
        return STATUS_SUCCESS;
    }
    UNICODE_STRING name = RTL_CONSTANT_STRING(PABULONG_CONTROL_DEVICE_NAME);
    UNICODE_STRING link = RTL_CONSTANT_STRING(PABULONG_CONTROL_SYMLINK);
    // SYSTEM and Administrators: all access. Interactive users: read/write
    // (the tray app runs as the signed-in user).
    UNICODE_STRING sddl = RTL_CONSTANT_STRING(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
    PDEVICE_OBJECT device = NULL;
    NTSTATUS status = IoCreateDeviceSecure(driver, 0, &name, FILE_DEVICE_UNKNOWN, FILE_DEVICE_SECURE_OPEN, FALSE, &sddl, &PabulongControlClass, &device);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = IoCreateSymbolicLink(&link, &name);
    if (!NT_SUCCESS(status)) {
        IoDeleteDevice(device);
        return status;
    }
    g.SymlinkCreated = TRUE;
    device->Flags &= ~DO_DEVICE_INITIALIZING;
    g.ControlDevice = device;
    return STATUS_SUCCESS;
}

void PabulongControlDelete()
{
    PAGED_CODE();
    if (g.SymlinkCreated) {
        UNICODE_STRING link = RTL_CONSTANT_STRING(PABULONG_CONTROL_SYMLINK);
        IoDeleteSymbolicLink(&link);
        g.SymlinkCreated = FALSE;
    }
    if (g.ControlDevice != NULL) {
        IoDeleteDevice(g.ControlDevice);
        g.ControlDevice = NULL;
    }
}

void PabulongStreamRunning(BOOLEAN running)
{
    PAGED_CODE();
    LONG count = running ? InterlockedIncrement(&g.Running) : InterlockedDecrement(&g.Running);
    if (count < 0) {
        InterlockedExchange(&g.Running, 0);
        count = 0;
    }
    if (g.Kernel != NULL) {
        InterlockedExchange(&g.Kernel->ConsumerRunningCount, count);
    }
}
#pragma code_seg()

PPABULONG_RING_HEADER PabulongRingHeader()
{
    return g.Kernel;
}
