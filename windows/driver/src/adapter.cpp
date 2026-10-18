// Driver entry, PnP and the two PortCls subdevices (wave + topology).
//
// Copyright (c) 2026 Next Matter Pro. MIT License.

#define PUT_GUIDS_HERE
#include "common.h"

extern "C" DRIVER_INITIALIZE DriverEntry;
DRIVER_ADD_DEVICE AddDevice;
DRIVER_UNLOAD PabulongUnload;
_Dispatch_type_(IRP_MJ_PNP) DRIVER_DISPATCH PnpHandler;

static PDRIVER_UNLOAD gPortClsUnload = NULL;
static PDRIVER_DISPATCH gPortClsPnp = NULL;

// One adapter, two subdevices.
#define MAX_MINIPORTS 2

// --------------------------------------------------------- new / delete

PVOID operator new(size_t size, POOL_FLAGS flags, ULONG tag)
{
    return ExAllocatePool2(flags, size, tag);  // zeroed unless POOL_FLAG_UNINITIALIZED
}

void __cdecl operator delete(PVOID p)
{
    if (p != NULL) {
        ExFreePool(p);
    }
}

void __cdecl operator delete(PVOID p, size_t)
{
    if (p != NULL) {
        ExFreePool(p);
    }
}

void __cdecl operator delete(PVOID p, ULONG tag)
{
    if (p != NULL) {
        ExFreePoolWithTag(p, tag);
    }
}

void __cdecl operator delete[](PVOID p)
{
    if (p != NULL) {
        ExFreePool(p);
    }
}

// ------------------------------------------------------------ subdevices

#pragma code_seg("PAGE")
static NTSTATUS InstallSubdevice(PDEVICE_OBJECT device, PIRP irp, PWSTR name, REFGUID portClass, PUNKNOWN miniport,
                                 PRESOURCELIST resources, PUNKNOWN* portOut)
{
    PAGED_CODE();
    PPORT port = NULL;
    NTSTATUS status = PcNewPort(&port, portClass);
    if (NT_SUCCESS(status)) {
        status = port->Init(device, irp, miniport, NULL, resources);
    }
    if (NT_SUCCESS(status)) {
        status = PcRegisterSubdevice(device, name, port);
    }
    if (NT_SUCCESS(status)) {
        status = port->QueryInterface(IID_IUnknown, (PVOID*)portOut);
    }
    if (port != NULL) {
        port->Release();
    }
    return status;
}

static NTSTATUS StartDevice(PDEVICE_OBJECT device, PIRP irp, PRESOURCELIST resources)
{
    PAGED_CODE();
    PUNKNOWN waveMiniport = NULL;
    PUNKNOWN topoMiniport = NULL;
    PUNKNOWN wavePort = NULL;
    PUNKNOWN topoPort = NULL;

    NTSTATUS status = CreateTopologyMiniport(&topoMiniport);
    if (NT_SUCCESS(status)) {
        status = InstallSubdevice(device, irp, (PWSTR)PABULONG_TOPOLOGY_NAME, CLSID_PortTopology, topoMiniport, resources, &topoPort);
    }
    if (NT_SUCCESS(status)) {
        status = CreateWaveMiniport(&waveMiniport);
    }
    if (NT_SUCCESS(status)) {
        status = InstallSubdevice(device, irp, (PWSTR)PABULONG_WAVE_NAME, CLSID_PortWaveRT, waveMiniport, resources, &wavePort);
    }
    if (NT_SUCCESS(status)) {
        // Capture path: topology bridge (out) -> wave bridge (in).
        status = PcRegisterPhysicalConnection(device, topoPort, PABULONG_TOPO_PIN_BRIDGE, wavePort, PABULONG_WAVE_PIN_BRIDGE);
    }
    if (NT_SUCCESS(status)) {
        status = PabulongControlCreate(device->DriverObject);
    }

    if (waveMiniport) waveMiniport->Release();
    if (topoMiniport) topoMiniport->Release();
    if (wavePort) wavePort->Release();
    if (topoPort) topoPort->Release();
    return status;
}

NTSTATUS AddDevice(PDRIVER_OBJECT driver, PDEVICE_OBJECT pdo)
{
    PAGED_CODE();
    return PcAddAdapterDevice(driver, pdo, PCPFNSTARTDEVICE(StartDevice), MAX_MINIPORTS, 0);
}

NTSTATUS PnpHandler(PDEVICE_OBJECT device, PIRP irp)
{
    PIO_STACK_LOCATION stack = IoGetCurrentIrpStackLocation(irp);
    if (stack->MinorFunction == IRP_MN_REMOVE_DEVICE) {
        // The control device must go before the driver can unload.
        PabulongControlDelete();
    }
    return gPortClsPnp(device, irp);
}

void PabulongUnload(PDRIVER_OBJECT driver)
{
    PAGED_CODE();
    PabulongControlDelete();
    if (gPortClsUnload != NULL) {
        gPortClsUnload(driver);
    }
    PabulongRingFree();
}
#pragma code_seg()

#pragma code_seg("INIT")
extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT driver, PUNICODE_STRING registryPath)
{
    NTSTATUS status = PabulongRingAllocate();
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = PcInitializeAdapterDriver(driver, registryPath, (PDRIVER_ADD_DEVICE)AddDevice);
    if (!NT_SUCCESS(status)) {
        PabulongRingFree();
        return status;
    }
    gPortClsPnp = driver->MajorFunction[IRP_MJ_PNP];
    driver->MajorFunction[IRP_MJ_PNP] = PnpHandler;
    PabulongControlHookDispatch(driver);
    gPortClsUnload = driver->DriverUnload;
    driver->DriverUnload = PabulongUnload;
    return STATUS_SUCCESS;
}
#pragma code_seg()
