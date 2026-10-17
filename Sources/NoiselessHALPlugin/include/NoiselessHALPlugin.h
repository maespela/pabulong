#pragma once

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

const char *nl_hal_virtual_device_name(void);
uint32_t nl_hal_nominal_sample_rate(void);
uint32_t nl_hal_nominal_frame_length(void);
bool nl_hal_reads_silence_without_producer(void);

#ifdef __cplusplus
}
#endif
