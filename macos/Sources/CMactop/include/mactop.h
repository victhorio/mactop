#ifndef MACTOP_H
#define MACTOP_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MactopHandle MactopHandle;

/* UTF-8 bytes; length-delimited, not necessarily NUL-terminated. */
typedef struct {
    const uint8_t *data;
    size_t len;
} MactopString;

typedef struct {
    uint32_t kind; /* 0 = Efficiency, 1 = Performance, 2 = Super. */
    const double *cores; /* Active residency percent per core, not frequency-scaled. */
    size_t core_count;
    double frequency_ghz; /* Active-weighted interval clock; NaN if unavailable. */
    double max_frequency_ghz; /* NaN if unavailable. */
} MactopCpuTier;

typedef struct {
    uint32_t pid;
    MactopString name;
    double cpu_percent; /* 100% per logical core; may exceed 100%. */
    uint64_t resident_bytes;
} MactopProcess;

typedef struct {
    uint64_t app_bytes;
    uint64_t wired_bytes;
    uint64_t compressed_bytes;
    uint64_t cached_bytes;
    uint64_t free_bytes;
    uint64_t swap_used_bytes;
    uint64_t swap_allocated_bytes;
    uint32_t pressure; /* 0 = Normal, 1 = Warning, 2 = Critical. */
    double swap_in_bytes_per_sec;
    double swap_out_bytes_per_sec;
} MactopMemory;

typedef struct {
    MactopString machine;
    const MactopCpuTier *tiers;
    size_t tier_count;
    MactopMemory memory;
    const MactopProcess *processes;
    size_t process_count;
    double time_seconds; /* Monotonic elapsed time from first Update.at; first = 0. */
    uint64_t sequence;
    uint8_t reset;
    uint8_t delayed;
} MactopSnapshot;

/*
 * Starts the shared Rust sampler (may block about one second for live metrics).
 * demo = 0 selects native metrics; nonzero selects fixtures. No automatic fallback.
 * Returns NULL on failure and writes bounded, NUL-terminated UTF-8 error text if
 * error != NULL and error_capacity > 0. The buffer must have error_capacity bytes.
 * On success the buffer is an empty string. NULL/zero-capacity buffers are allowed.
 * The initial reading is retained for the first successful poll.
 * The caller owns the handle and must destroy it exactly once.
 */
MactopHandle *mactop_start(uint8_t demo, char *error, size_t error_capacity);

/*
 * Returns 1 = new reading, 0 = no new reading, -1 = error. Only 1 writes *out.
 * out must point to aligned, writable MactopSnapshot storage (NULL returns -1).
 * No fabricated snapshot is returned on error; later polls can recover from
 * collection errors. Use mactop_last_error for the current error message.
 * Every pointer in a successful snapshot borrows handle-owned storage, valid
 * until the next poll (regardless of result) or destroy. Copy any data to keep.
 * Arrays/strings are bounded by their counts/lengths; never free their pointers.
 * All tiers, cores, processes, full names, and available clocks are preserved.
 * A single consumer must serialize all calls for a handle.
 * A NULL handle returns -1; other nonnull handles must be live and valid.
 */
int32_t mactop_poll(MactopHandle *handle, MactopSnapshot *out);

/*
 * Changes collection cadence without restarting counters. Supported intervals
 * are 250, 500, 1000, 2000, and 5000 milliseconds (4, 2, 1, 0.5, and 0.2 Hz).
 * Returns 0 = command accepted, -1 = error. NULL returns -1. The command applies
 * asynchronously on the sampling worker; pending snapshots remain valid.
 * Calls must be serialized with poll, last_error, and destroy.
 */
int32_t mactop_set_interval(MactopHandle *handle, uint32_t interval_milliseconds);

/*
 * Borrowed NUL-terminated UTF-8 text, empty if no error. Valid until the next
 * poll/set_interval/destroy; never free it. Poll and set_interval clear the
 * previous error before trying again.
 * NULL returns static "invalid mactop handle" text.
 */
const char *mactop_last_error(const MactopHandle *handle);

/* Stops/joins the worker and releases its native handles on that worker thread.
 * NULL is a no-op. The handle and all its borrowed pointers are invalid afterward.
 */
void mactop_destroy(MactopHandle *handle);

#ifdef __cplusplus
}
#endif

#endif
