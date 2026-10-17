#include "NoiselessHALPlugin.h"
#include "NoiselessAudioCoreC.h"

#include <CoreAudio/AudioHardware.h>
#include <CoreAudio/AudioServerPlugIn.h>
#include <CoreFoundation/CoreFoundation.h>
#include <mach/mach_time.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#ifndef kAudioObjectPropertyElementMain
#define kAudioObjectPropertyElementMain kAudioObjectPropertyElementMaster
#endif

enum {
    kNoiselessObjectPlugIn = kAudioObjectPlugInObject,
    kNoiselessObjectDevice = 2,
    kNoiselessObjectStreamInput = 3,
};

static const Float64 kNoiselessSampleRate = 48000.0;
static const UInt32 kNoiselessFrameLength = 480;
static const UInt32 kNoiselessBufferFrameSize = 480;
static const UInt32 kNoiselessZeroTimeStampPeriod = 16384;
static const UInt32 kNoiselessMaxBufferedFrames = 3;
static const UInt32 kNoiselessChannels = 1;
static const char *kNoiselessBundleID = "com.nextmatterpro.pabulong.driver";
static const char *kNoiselessDeviceUID = "com.nextmatterpro.pabulong.mic.device";
static const char *kNoiselessModelUID = "com.nextmatterpro.pabulong.mic.model";

static AudioServerPlugInHostRef gHost = NULL;
static atomic_uint gRefCount = 1;
static atomic_uint gRunningClients = 0;
static UInt64 gZeroHostTime = 0;
static UInt64 gZeroTimeStampCount = 0;
static Float64 gPreviousZeroTicks = 0;
static Float64 gHostTicksPerFrame = 0;
static UInt64 gZeroSeed = 1;
static NLSharedRing *gSharedRing = NULL;
static Float32 gCurrentFrame[480];
static UInt32 gCurrentFrameOffset = 480;

static HRESULT Noiseless_QueryInterface(void *driver, REFIID uuid, LPVOID *outInterface);
static ULONG Noiseless_AddRef(void *driver);
static ULONG Noiseless_Release(void *driver);
static OSStatus Noiseless_Initialize(AudioServerPlugInDriverRef driver, AudioServerPlugInHostRef host);
static OSStatus Noiseless_CreateDevice(AudioServerPlugInDriverRef driver, CFDictionaryRef description, const AudioServerPlugInClientInfo *clientInfo, AudioObjectID *outDeviceObjectID);
static OSStatus Noiseless_DestroyDevice(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID);
static OSStatus Noiseless_AddDeviceClient(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, const AudioServerPlugInClientInfo *clientInfo);
static OSStatus Noiseless_RemoveDeviceClient(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, const AudioServerPlugInClientInfo *clientInfo);
static OSStatus Noiseless_PerformDeviceConfigurationChange(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt64 changeAction, void *changeInfo);
static OSStatus Noiseless_AbortDeviceConfigurationChange(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt64 changeAction, void *changeInfo);
static Boolean Noiseless_HasProperty(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address);
static OSStatus Noiseless_IsPropertySettable(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, Boolean *outIsSettable);
static OSStatus Noiseless_GetPropertyDataSize(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, UInt32 qualifierDataSize, const void *qualifierData, UInt32 *outDataSize);
static OSStatus Noiseless_GetPropertyData(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, UInt32 qualifierDataSize, const void *qualifierData, UInt32 dataSize, UInt32 *outDataSize, void *outData);
static OSStatus Noiseless_SetPropertyData(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, UInt32 qualifierDataSize, const void *qualifierData, UInt32 dataSize, const void *data);
static OSStatus Noiseless_StartIO(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID);
static OSStatus Noiseless_StopIO(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID);
static OSStatus Noiseless_GetZeroTimeStamp(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, Float64 *outSampleTime, UInt64 *outHostTime, UInt64 *outSeed);
static OSStatus Noiseless_WillDoIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, UInt32 operationID, Boolean *outWillDo, Boolean *outWillDoInPlace);
static OSStatus Noiseless_BeginIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, UInt32 operationID, UInt32 ioBufferFrameSize, const AudioServerPlugInIOCycleInfo *ioCycleInfo);
static OSStatus Noiseless_DoIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, AudioObjectID streamObjectID, UInt32 clientID, UInt32 operationID, UInt32 ioBufferFrameSize, const AudioServerPlugInIOCycleInfo *ioCycleInfo, void *ioMainBuffer, void *ioSecondaryBuffer);
static OSStatus Noiseless_EndIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, UInt32 operationID, UInt32 ioBufferFrameSize, const AudioServerPlugInIOCycleInfo *ioCycleInfo);

static AudioServerPlugInDriverInterface gDriverInterface = {
    NULL,
    Noiseless_QueryInterface,
    Noiseless_AddRef,
    Noiseless_Release,
    Noiseless_Initialize,
    Noiseless_CreateDevice,
    Noiseless_DestroyDevice,
    Noiseless_AddDeviceClient,
    Noiseless_RemoveDeviceClient,
    Noiseless_PerformDeviceConfigurationChange,
    Noiseless_AbortDeviceConfigurationChange,
    Noiseless_HasProperty,
    Noiseless_IsPropertySettable,
    Noiseless_GetPropertyDataSize,
    Noiseless_GetPropertyData,
    Noiseless_SetPropertyData,
    Noiseless_StartIO,
    Noiseless_StopIO,
    Noiseless_GetZeroTimeStamp,
    Noiseless_WillDoIOOperation,
    Noiseless_BeginIOOperation,
    Noiseless_DoIOOperation,
    Noiseless_EndIOOperation,
};

static AudioServerPlugInDriverInterface *gDriverInterfacePtr = &gDriverInterface;
static AudioServerPlugInDriverRef gDriverRef = &gDriverInterfacePtr;

const char *nl_hal_virtual_device_name(void) { return "Pabulong Mic"; }
uint32_t nl_hal_nominal_sample_rate(void) { return 48000; }
uint32_t nl_hal_nominal_frame_length(void) { return 480; }
bool nl_hal_reads_silence_without_producer(void) { return true; }

void *Pabulong_Create(CFAllocatorRef allocator, CFUUIDRef requestedTypeUUID) {
    (void)allocator;
    if (CFEqual(requestedTypeUUID, kAudioServerPlugInTypeUUID)) {
        return gDriverRef;
    }
    return NULL;
}

static AudioStreamBasicDescription noiseless_asbd(void) {
    AudioStreamBasicDescription asbd;
    memset(&asbd, 0, sizeof(asbd));
    asbd.mSampleRate = kNoiselessSampleRate;
    asbd.mFormatID = kAudioFormatLinearPCM;
    asbd.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagsNativeEndian;
    asbd.mBytesPerPacket = sizeof(Float32) * kNoiselessChannels;
    asbd.mFramesPerPacket = 1;
    asbd.mBytesPerFrame = sizeof(Float32) * kNoiselessChannels;
    asbd.mChannelsPerFrame = kNoiselessChannels;
    asbd.mBitsPerChannel = 32;
    return asbd;
}

static AudioStreamRangedDescription noiseless_ranged_asbd(void) {
    AudioStreamRangedDescription ranged;
    ranged.mFormat = noiseless_asbd();
    ranged.mSampleRateRange.mMinimum = kNoiselessSampleRate;
    ranged.mSampleRateRange.mMaximum = kNoiselessSampleRate;
    return ranged;
}

static UInt32 object_class(AudioObjectID objectID) {
    switch (objectID) {
    case kNoiselessObjectPlugIn: return kAudioPlugInClassID;
    case kNoiselessObjectDevice: return kAudioDeviceClassID;
    case kNoiselessObjectStreamInput: return kAudioStreamClassID;
    default: return 0;
    }
}

static AudioObjectID object_owner(AudioObjectID objectID) {
    switch (objectID) {
    case kNoiselessObjectPlugIn: return kAudioObjectUnknown;
    case kNoiselessObjectDevice: return kNoiselessObjectPlugIn;
    case kNoiselessObjectStreamInput: return kNoiselessObjectDevice;
    default: return kAudioObjectUnknown;
    }
}

static bool is_known_object(AudioObjectID objectID) {
    return objectID == kNoiselessObjectPlugIn || objectID == kNoiselessObjectDevice || objectID == kNoiselessObjectStreamInput;
}

static bool class_matches(AudioClassID objectClass, UInt32 qualifierDataSize, const void *qualifierData) {
    if (qualifierDataSize < sizeof(AudioClassID) || !qualifierData) {
        return true;
    }

    const UInt32 classCount = qualifierDataSize / sizeof(AudioClassID);
    const AudioClassID *classes = (const AudioClassID *)qualifierData;
    for (UInt32 index = 0; index < classCount; ++index) {
        if (classes[index] == kAudioObjectClassIDWildcard || classes[index] == objectClass || classes[index] == kAudioObjectClassID) {
            return true;
        }
    }
    return false;
}

static UInt32 owned_object_for(AudioObjectID owner, UInt32 qualifierDataSize, const void *qualifierData, AudioObjectID *objectID) {
    if (!objectID) { return 0; }
    if (owner == kNoiselessObjectPlugIn && class_matches(kAudioDeviceClassID, qualifierDataSize, qualifierData)) {
        *objectID = kNoiselessObjectDevice;
        return 1;
    }
    if (owner == kNoiselessObjectDevice && class_matches(kAudioStreamClassID, qualifierDataSize, qualifierData)) {
        *objectID = kNoiselessObjectStreamInput;
        return 1;
    }
    return 0;
}

static OSStatus copy_data(const void *source, UInt32 sourceSize, UInt32 dataSize, UInt32 *outDataSize, void *outData) {
    if (!outDataSize || !outData) { return kAudioHardwareIllegalOperationError; }
    if (dataSize < sourceSize) { return kAudioHardwareBadPropertySizeError; }
    memcpy(outData, source, sourceSize);
    *outDataSize = sourceSize;
    return noErr;
}

static OSStatus copy_u32(UInt32 value, UInt32 dataSize, UInt32 *outDataSize, void *outData) {
    return copy_data(&value, sizeof(value), dataSize, outDataSize, outData);
}

static OSStatus copy_f64(Float64 value, UInt32 dataSize, UInt32 *outDataSize, void *outData) {
    return copy_data(&value, sizeof(value), dataSize, outDataSize, outData);
}

static OSStatus copy_object_id(AudioObjectID value, UInt32 dataSize, UInt32 *outDataSize, void *outData) {
    return copy_data(&value, sizeof(value), dataSize, outDataSize, outData);
}

static OSStatus copy_cf_string(const char *value, UInt32 dataSize, UInt32 *outDataSize, void *outData) {
    if (!outDataSize || !outData) { return kAudioHardwareIllegalOperationError; }
    if (dataSize < sizeof(CFStringRef)) { return kAudioHardwareBadPropertySizeError; }
    CFStringRef string = CFStringCreateWithCString(kCFAllocatorDefault, value, kCFStringEncodingUTF8);
    *((CFStringRef *)outData) = string;
    *outDataSize = sizeof(CFStringRef);
    return noErr;
}

static OSStatus copy_stream_configuration(UInt32 dataSize, UInt32 *outDataSize, void *outData, bool hasInput) {
    const UInt32 required = offsetof(AudioBufferList, mBuffers) + sizeof(AudioBuffer);
    if (!outDataSize || !outData) { return kAudioHardwareIllegalOperationError; }
    if (dataSize < required) { return kAudioHardwareBadPropertySizeError; }
    AudioBufferList *list = (AudioBufferList *)outData;
    list->mNumberBuffers = 1;
    list->mBuffers[0].mNumberChannels = hasInput ? kNoiselessChannels : 0;
    list->mBuffers[0].mDataByteSize = 0;
    list->mBuffers[0].mData = NULL;
    *outDataSize = required;
    return noErr;
}

static void ensure_shared_ring(void) {
    if (!gSharedRing) {
        // Create the ring from the HAL control path as well as from the app. If a
        // meeting app starts IO before the menu app has opened the segment, the
        // running-client count must still be published for first-try activation.
        gSharedRing = nl_shared_ring_open(nl_shared_ring_default_name(), true, 32, kNoiselessFrameLength);
    }
}

static void publish_running_clients(void) {
    if (gSharedRing) {
        nl_shared_ring_set_consumer_running_count(gSharedRing, atomic_load(&gRunningClients));
    }
}

static Float64 calculate_host_ticks_per_frame(void) {
    struct mach_timebase_info info;
    mach_timebase_info(&info);
    Float64 hostClockFrequency = (Float64)info.denom / (Float64)info.numer;
    hostClockFrequency *= 1000000000.0;
    return hostClockFrequency / kNoiselessSampleRate;
}

static void reset_zero_time_stamp(void) {
    gZeroHostTime = mach_absolute_time();
    gZeroTimeStampCount = 0;
    gPreviousZeroTicks = 0;
    gHostTicksPerFrame = calculate_host_ticks_per_frame();
    gZeroSeed++;
}

static void read_shared_sample_block(Float32 *output, UInt32 frameCount) {
    // Real-time IO thread: never open shared memory here (shm_open is a blocking
    // syscall). The ring is opened on the StartIO control path; until it exists
    // we emit silence.
    for (UInt32 frame = 0; frame < frameCount; ++frame) {
        if (gCurrentFrameOffset >= kNoiselessFrameLength) {
            if (gSharedRing) {
                nl_shared_ring_read_latest(gSharedRing, gCurrentFrame, kNoiselessFrameLength, kNoiselessMaxBufferedFrames);
            } else {
                memset(gCurrentFrame, 0, sizeof(gCurrentFrame));
            }
            gCurrentFrameOffset = 0;
        }
        output[frame] = gCurrentFrame[gCurrentFrameOffset++];
    }
}

static HRESULT Noiseless_QueryInterface(void *driver, REFIID uuid, LPVOID *outInterface) {
    if (driver != gDriverRef || !outInterface) { return kAudioHardwareBadObjectError; }
    CFUUIDRef requestedUUID = CFUUIDCreateFromUUIDBytes(NULL, uuid);
    if (!requestedUUID) { return kAudioHardwareIllegalOperationError; }
    HRESULT result = E_NOINTERFACE;
    if (CFEqual(requestedUUID, IUnknownUUID) || CFEqual(requestedUUID, kAudioServerPlugInDriverInterfaceUUID)) {
        atomic_fetch_add(&gRefCount, 1);
        *outInterface = gDriverRef;
        result = noErr;
    }
    CFRelease(requestedUUID);
    return result;
}

static ULONG Noiseless_AddRef(void *driver) {
    if (driver != gDriverRef) { return 0; }
    return atomic_fetch_add(&gRefCount, 1) + 1;
}

static ULONG Noiseless_Release(void *driver) {
    if (driver != gDriverRef) { return 0; }
    unsigned int current = atomic_load(&gRefCount);
    while (current > 0 && !atomic_compare_exchange_weak(&gRefCount, &current, current - 1)) {}
    return current > 0 ? current - 1 : 0;
}

static OSStatus Noiseless_Initialize(AudioServerPlugInDriverRef driver, AudioServerPlugInHostRef host) {
    if (driver != gDriverRef) { return kAudioHardwareBadObjectError; }
    gHost = host;
    reset_zero_time_stamp();
    ensure_shared_ring();
    return noErr;
}

static OSStatus Noiseless_CreateDevice(AudioServerPlugInDriverRef driver, CFDictionaryRef description, const AudioServerPlugInClientInfo *clientInfo, AudioObjectID *outDeviceObjectID) {
    (void)description; (void)clientInfo;
    if (driver != gDriverRef) { return kAudioHardwareBadObjectError; }
    if (outDeviceObjectID) { *outDeviceObjectID = kNoiselessObjectDevice; }
    return noErr;
}

static OSStatus Noiseless_DestroyDevice(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID) {
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}

static OSStatus Noiseless_AddDeviceClient(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, const AudioServerPlugInClientInfo *clientInfo) {
    (void)clientInfo;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}

static OSStatus Noiseless_RemoveDeviceClient(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, const AudioServerPlugInClientInfo *clientInfo) {
    (void)clientInfo;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}

static OSStatus Noiseless_PerformDeviceConfigurationChange(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt64 changeAction, void *changeInfo) {
    (void)changeAction; (void)changeInfo;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}

static OSStatus Noiseless_AbortDeviceConfigurationChange(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt64 changeAction, void *changeInfo) {
    (void)changeAction; (void)changeInfo;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}

static Boolean Noiseless_HasProperty(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address) {
    (void)clientProcessID;
    if (driver != gDriverRef || !address || !is_known_object(objectID)) { return false; }
    switch (address->mSelector) {
    case kAudioObjectPropertyBaseClass:
    case kAudioObjectPropertyClass:
    case kAudioObjectPropertyOwner:
    case kAudioObjectPropertyName:
    case kAudioObjectPropertyModelName:
    case kAudioObjectPropertyManufacturer:
    case kAudioObjectPropertyElementName:
    case kAudioObjectPropertyElementCategoryName:
    case kAudioObjectPropertyElementNumberName:
    case kAudioObjectPropertyOwnedObjects:
    case kAudioObjectPropertyCustomPropertyInfoList:
        return true;
    case kAudioPlugInPropertyBundleID:
    case kAudioPlugInPropertyResourceBundle:
    case kAudioPlugInPropertyDeviceList:
    case kAudioPlugInPropertyTranslateUIDToDevice:
    case kAudioPlugInPropertyBoxList:
    case kAudioPlugInPropertyClockDeviceList:
        return objectID == kNoiselessObjectPlugIn;
    case kAudioDevicePropertyDeviceUID:
    case kAudioDevicePropertyModelUID:
    case kAudioDevicePropertyTransportType:
    case kAudioDevicePropertyRelatedDevices:
    case kAudioDevicePropertyClockDomain:
    case kAudioDevicePropertyDeviceIsAlive:
    case kAudioDevicePropertyDeviceIsRunning:
    case kAudioDevicePropertyDeviceCanBeDefaultDevice:
    case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice:
    case kAudioDevicePropertyNominalSampleRate:
    case kAudioDevicePropertyActualSampleRate:
    case kAudioDevicePropertyAvailableNominalSampleRates:
    case kAudioDevicePropertyIsHidden:
    case kAudioDevicePropertyPreferredChannelsForStereo:
    case kAudioDevicePropertyStreams:
    case kAudioDevicePropertyStreamConfiguration:
    case kAudioObjectPropertyControlList:
    case kAudioDevicePropertyLatency:
    case kAudioDevicePropertySafetyOffset:
    case kAudioDevicePropertyBufferFrameSize:
    case kAudioDevicePropertyBufferFrameSizeRange:
    case kAudioDevicePropertyUsesVariableBufferFrameSizes:
    case kAudioDevicePropertyZeroTimeStampPeriod:
    case kAudioDevicePropertyHogMode:
    case kAudioDevicePropertyProcessMute:
        return objectID == kNoiselessObjectDevice || (address->mSelector == kAudioDevicePropertyLatency && objectID == kNoiselessObjectStreamInput);
    case kAudioStreamPropertyIsActive:
    case kAudioStreamPropertyDirection:
    case kAudioStreamPropertyTerminalType:
    case kAudioStreamPropertyStartingChannel:
    case kAudioStreamPropertyVirtualFormat:
    case kAudioStreamPropertyAvailableVirtualFormats:
    case kAudioStreamPropertyPhysicalFormat:
    case kAudioStreamPropertyAvailablePhysicalFormats:
        return objectID == kNoiselessObjectStreamInput;
    default:
        return false;
    }
}

static OSStatus Noiseless_IsPropertySettable(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, Boolean *outIsSettable) {
    (void)clientProcessID;
    if (driver != gDriverRef || !address || !outIsSettable || !is_known_object(objectID)) { return kAudioHardwareBadObjectError; }
    *outIsSettable = false;
    return Noiseless_HasProperty(driver, objectID, clientProcessID, address) ? noErr : kAudioHardwareUnknownPropertyError;
}

static OSStatus Noiseless_GetPropertyDataSize(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, UInt32 qualifierDataSize, const void *qualifierData, UInt32 *outDataSize) {
    (void)qualifierDataSize; (void)qualifierData;
    if (driver != gDriverRef || !address || !outDataSize || !Noiseless_HasProperty(driver, objectID, clientProcessID, address)) { return kAudioHardwareUnknownPropertyError; }
    switch (address->mSelector) {
    case kAudioObjectPropertyName:
    case kAudioObjectPropertyModelName:
    case kAudioObjectPropertyManufacturer:
    case kAudioObjectPropertyElementName:
    case kAudioObjectPropertyElementCategoryName:
    case kAudioObjectPropertyElementNumberName:
    case kAudioPlugInPropertyBundleID:
    case kAudioDevicePropertyDeviceUID:
    case kAudioDevicePropertyModelUID:
        *outDataSize = sizeof(CFStringRef); return noErr;
    case kAudioObjectPropertyOwnedObjects:
        {
            AudioObjectID owned = kAudioObjectUnknown;
            *outDataSize = owned_object_for(objectID, qualifierDataSize, qualifierData, &owned) * sizeof(AudioObjectID);
            return noErr;
        }
    case kAudioObjectPropertyCustomPropertyInfoList:
    case kAudioObjectPropertyControlList:
        *outDataSize = 0; return noErr;
    case kAudioPlugInPropertyResourceBundle:
        *outDataSize = sizeof(CFStringRef); return noErr;
    case kAudioPlugInPropertyDeviceList:
        *outDataSize = sizeof(AudioObjectID); return noErr;
    case kAudioPlugInPropertyTranslateUIDToDevice:
        *outDataSize = sizeof(AudioObjectID); return noErr;
    case kAudioPlugInPropertyBoxList:
    case kAudioPlugInPropertyClockDeviceList:
        *outDataSize = 0; return noErr;
    case kAudioDevicePropertyAvailableNominalSampleRates:
    case kAudioDevicePropertyBufferFrameSizeRange:
        *outDataSize = sizeof(AudioValueRange); return noErr;
    case kAudioDevicePropertyRelatedDevices:
        *outDataSize = sizeof(AudioObjectID); return noErr;
    case kAudioDevicePropertyPreferredChannelsForStereo:
        *outDataSize = sizeof(UInt32) * 2; return noErr;
    case kAudioDevicePropertyStreams:
        *outDataSize = address->mScope == kAudioObjectPropertyScopeInput ? sizeof(AudioObjectID) : 0; return noErr;
    case kAudioDevicePropertyStreamConfiguration:
        *outDataSize = offsetof(AudioBufferList, mBuffers) + sizeof(AudioBuffer); return noErr;
    case kAudioStreamPropertyVirtualFormat:
    case kAudioStreamPropertyPhysicalFormat:
        *outDataSize = sizeof(AudioStreamBasicDescription); return noErr;
    case kAudioStreamPropertyAvailableVirtualFormats:
    case kAudioStreamPropertyAvailablePhysicalFormats:
        *outDataSize = sizeof(AudioStreamRangedDescription); return noErr;
    case kAudioDevicePropertyNominalSampleRate:
    case kAudioDevicePropertyActualSampleRate:
        *outDataSize = sizeof(Float64); return noErr;
    case kAudioDevicePropertyHogMode:
        *outDataSize = sizeof(pid_t); return noErr;
    default:
        *outDataSize = sizeof(UInt32); return noErr;
    }
}

static OSStatus Noiseless_GetPropertyData(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, UInt32 qualifierDataSize, const void *qualifierData, UInt32 dataSize, UInt32 *outDataSize, void *outData) {
    if (driver != gDriverRef || !address || !Noiseless_HasProperty(driver, objectID, clientProcessID, address)) { return kAudioHardwareUnknownPropertyError; }
    switch (address->mSelector) {
    case kAudioObjectPropertyBaseClass: return copy_u32(objectID == kNoiselessObjectPlugIn ? kAudioPlugInClassID : kAudioObjectClassID, dataSize, outDataSize, outData);
    case kAudioObjectPropertyClass: return copy_u32(object_class(objectID), dataSize, outDataSize, outData);
    case kAudioObjectPropertyOwner: return copy_object_id(object_owner(objectID), dataSize, outDataSize, outData);
    case kAudioObjectPropertyName:
        return copy_cf_string(objectID == kNoiselessObjectStreamInput ? "Pabulong Mic Stream" : "Pabulong Mic", dataSize, outDataSize, outData);
    case kAudioObjectPropertyModelName:
        return copy_cf_string("Pabulong Virtual Microphone", dataSize, outDataSize, outData);
    case kAudioObjectPropertyManufacturer: return copy_cf_string("Next Matter Pro", dataSize, outDataSize, outData);
    case kAudioObjectPropertyElementName:
        return copy_cf_string("Pabulong Mic", dataSize, outDataSize, outData);
    case kAudioObjectPropertyElementCategoryName:
        return copy_cf_string("Input", dataSize, outDataSize, outData);
    case kAudioObjectPropertyElementNumberName:
        return copy_cf_string("1", dataSize, outDataSize, outData);
    case kAudioObjectPropertyOwnedObjects:
        {
            AudioObjectID owned = kAudioObjectUnknown;
            UInt32 count = owned_object_for(objectID, qualifierDataSize, qualifierData, &owned);
            if (count == 0) { *outDataSize = 0; return noErr; }
            return copy_object_id(owned, dataSize, outDataSize, outData);
        }
    case kAudioObjectPropertyCustomPropertyInfoList:
    case kAudioObjectPropertyControlList:
        *outDataSize = 0; return noErr;
    case kAudioPlugInPropertyBundleID: return copy_cf_string(kNoiselessBundleID, dataSize, outDataSize, outData);
    case kAudioPlugInPropertyResourceBundle: return copy_cf_string(kNoiselessBundleID, dataSize, outDataSize, outData);
    case kAudioPlugInPropertyDeviceList: return copy_object_id(kNoiselessObjectDevice, dataSize, outDataSize, outData);
    case kAudioPlugInPropertyTranslateUIDToDevice: {
        AudioObjectID translated = kAudioObjectUnknown;
        if (qualifierDataSize == sizeof(CFStringRef) && qualifierData) {
            CFStringRef uid = *((CFStringRef const *)qualifierData);
            if (uid && CFStringCompare(uid, CFSTR("com.nextmatterpro.pabulong.mic.device"), 0) == kCFCompareEqualTo) {
                translated = kNoiselessObjectDevice;
            }
        }
        return copy_object_id(translated, dataSize, outDataSize, outData);
    }
    case kAudioPlugInPropertyBoxList:
    case kAudioPlugInPropertyClockDeviceList:
        *outDataSize = 0; return noErr;
    case kAudioDevicePropertyDeviceUID: return copy_cf_string(kNoiselessDeviceUID, dataSize, outDataSize, outData);
    case kAudioDevicePropertyModelUID: return copy_cf_string(kNoiselessModelUID, dataSize, outDataSize, outData);
    case kAudioDevicePropertyTransportType: return copy_u32(kAudioDeviceTransportTypeVirtual, dataSize, outDataSize, outData);
    case kAudioDevicePropertyRelatedDevices: return copy_object_id(kNoiselessObjectDevice, dataSize, outDataSize, outData);
    case kAudioDevicePropertyClockDomain: return copy_u32(0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyDeviceIsAlive: return copy_u32(1, dataSize, outDataSize, outData);
    case kAudioDevicePropertyDeviceIsRunning: return copy_u32(atomic_load(&gRunningClients) > 0 ? 1 : 0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyDeviceCanBeDefaultDevice: return copy_u32(address->mScope == kAudioObjectPropertyScopeInput ? 1 : 0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice: return copy_u32(0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyNominalSampleRate:
    case kAudioDevicePropertyActualSampleRate:
        return copy_f64(kNoiselessSampleRate, dataSize, outDataSize, outData);
    case kAudioDevicePropertyAvailableNominalSampleRates: {
        AudioValueRange range = { kNoiselessSampleRate, kNoiselessSampleRate };
        return copy_data(&range, sizeof(range), dataSize, outDataSize, outData);
    }
    case kAudioDevicePropertyStreams:
        if (address->mScope == kAudioObjectPropertyScopeInput) { return copy_object_id(kNoiselessObjectStreamInput, dataSize, outDataSize, outData); }
        *outDataSize = 0; return noErr;
    case kAudioDevicePropertyIsHidden:
        return copy_u32(0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyPreferredChannelsForStereo: {
        UInt32 channels[2] = { 1, 1 };
        return copy_data(channels, sizeof(channels), dataSize, outDataSize, outData);
    }
    case kAudioDevicePropertyStreamConfiguration:
        return copy_stream_configuration(dataSize, outDataSize, outData, address->mScope == kAudioObjectPropertyScopeInput);
    case kAudioDevicePropertyLatency:
    case kAudioDevicePropertySafetyOffset:
        return copy_u32(0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyBufferFrameSize:
        return copy_u32(kNoiselessBufferFrameSize, dataSize, outDataSize, outData);
    case kAudioDevicePropertyBufferFrameSizeRange: {
        AudioValueRange range = { kNoiselessBufferFrameSize, kNoiselessBufferFrameSize };
        return copy_data(&range, sizeof(range), dataSize, outDataSize, outData);
    }
    case kAudioDevicePropertyUsesVariableBufferFrameSizes:
        return copy_u32(0, dataSize, outDataSize, outData);
    case kAudioDevicePropertyZeroTimeStampPeriod:
        return copy_u32(kNoiselessZeroTimeStampPeriod, dataSize, outDataSize, outData);
    case kAudioDevicePropertyHogMode: {
        pid_t hog = -1;
        return copy_data(&hog, sizeof(hog), dataSize, outDataSize, outData);
    }
    case kAudioDevicePropertyProcessMute:
        return copy_u32(0, dataSize, outDataSize, outData);
    case kAudioStreamPropertyIsActive:
        return copy_u32(1, dataSize, outDataSize, outData);
    case kAudioStreamPropertyDirection:
        return copy_u32(1, dataSize, outDataSize, outData);
    case kAudioStreamPropertyTerminalType:
        return copy_u32(kAudioStreamTerminalTypeMicrophone, dataSize, outDataSize, outData);
    case kAudioStreamPropertyStartingChannel:
        return copy_u32(1, dataSize, outDataSize, outData);
    case kAudioStreamPropertyVirtualFormat:
    case kAudioStreamPropertyPhysicalFormat: {
        AudioStreamBasicDescription asbd = noiseless_asbd();
        return copy_data(&asbd, sizeof(asbd), dataSize, outDataSize, outData);
    }
    case kAudioStreamPropertyAvailableVirtualFormats:
    case kAudioStreamPropertyAvailablePhysicalFormats: {
        AudioStreamRangedDescription ranged = noiseless_ranged_asbd();
        return copy_data(&ranged, sizeof(ranged), dataSize, outDataSize, outData);
    }
    default:
        return kAudioHardwareUnknownPropertyError;
    }
}

static OSStatus Noiseless_SetPropertyData(AudioServerPlugInDriverRef driver, AudioObjectID objectID, pid_t clientProcessID, const AudioObjectPropertyAddress *address, UInt32 qualifierDataSize, const void *qualifierData, UInt32 dataSize, const void *data) {
    (void)objectID; (void)clientProcessID; (void)address; (void)qualifierDataSize; (void)qualifierData; (void)dataSize; (void)data;
    return driver == gDriverRef ? kAudioHardwareIllegalOperationError : kAudioHardwareBadObjectError;
}

static OSStatus Noiseless_StartIO(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID) {
    (void)clientID;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    atomic_fetch_add(&gRunningClients, 1);
    reset_zero_time_stamp();
    // Open the shared ring on this control path so the real-time IO thread never
    // has to make blocking syscalls.
    ensure_shared_ring();
    publish_running_clients();
    return noErr;
}

static OSStatus Noiseless_StopIO(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID) {
    (void)clientID;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    unsigned int current = atomic_load(&gRunningClients);
    while (current > 0 && !atomic_compare_exchange_weak(&gRunningClients, &current, current - 1)) {}
    publish_running_clients();
    return noErr;
}

static OSStatus Noiseless_GetZeroTimeStamp(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, Float64 *outSampleTime, UInt64 *outHostTime, UInt64 *outSeed) {
    (void)clientID;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    if (!outSampleTime || !outHostTime || !outSeed) { return kAudioHardwareIllegalOperationError; }
    const UInt64 currentHostTime = mach_absolute_time();
    const Float64 ticksPerPeriod = gHostTicksPerFrame * (Float64)kNoiselessZeroTimeStampPeriod;
    Float64 nextTickOffset = gPreviousZeroTicks + ticksPerPeriod;
    UInt64 nextHostTime = gZeroHostTime + (UInt64)nextTickOffset;
    while (nextHostTime <= currentHostTime) {
        gZeroTimeStampCount++;
        gPreviousZeroTicks = nextTickOffset;
        nextTickOffset = gPreviousZeroTicks + ticksPerPeriod;
        nextHostTime = gZeroHostTime + (UInt64)nextTickOffset;
    }

    *outSampleTime = (Float64)(gZeroTimeStampCount * kNoiselessZeroTimeStampPeriod);
    *outHostTime = gZeroHostTime + (UInt64)gPreviousZeroTicks;
    *outSeed = gZeroSeed;
    return noErr;
}

static OSStatus Noiseless_WillDoIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, UInt32 operationID, Boolean *outWillDo, Boolean *outWillDoInPlace) {
    (void)clientID;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    if (outWillDo) { *outWillDo = operationID == kAudioServerPlugInIOOperationReadInput; }
    if (outWillDoInPlace) { *outWillDoInPlace = true; }
    return noErr;
}

static OSStatus Noiseless_BeginIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, UInt32 operationID, UInt32 ioBufferFrameSize, const AudioServerPlugInIOCycleInfo *ioCycleInfo) {
    (void)clientID; (void)operationID; (void)ioBufferFrameSize; (void)ioCycleInfo;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}

static OSStatus Noiseless_DoIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, AudioObjectID streamObjectID, UInt32 clientID, UInt32 operationID, UInt32 ioBufferFrameSize, const AudioServerPlugInIOCycleInfo *ioCycleInfo, void *ioMainBuffer, void *ioSecondaryBuffer) {
    (void)clientID; (void)ioCycleInfo; (void)ioSecondaryBuffer;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice || streamObjectID != kNoiselessObjectStreamInput) { return kAudioHardwareBadObjectError; }
    if (operationID != kAudioServerPlugInIOOperationReadInput) { return noErr; }
    if (!ioMainBuffer) { return kAudioHardwareIllegalOperationError; }
    read_shared_sample_block((Float32 *)ioMainBuffer, ioBufferFrameSize);
    return noErr;
}

static OSStatus Noiseless_EndIOOperation(AudioServerPlugInDriverRef driver, AudioObjectID deviceObjectID, UInt32 clientID, UInt32 operationID, UInt32 ioBufferFrameSize, const AudioServerPlugInIOCycleInfo *ioCycleInfo) {
    (void)clientID; (void)operationID; (void)ioBufferFrameSize; (void)ioCycleInfo;
    if (driver != gDriverRef || deviceObjectID != kNoiselessObjectDevice) { return kAudioHardwareBadObjectError; }
    return noErr;
}
