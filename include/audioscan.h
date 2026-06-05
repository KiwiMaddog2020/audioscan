/*
* Analyze audio files with audioscan.
*
* audioscan_analyze_json returns a newly allocated JSON string on success and NULL
* on hard errors, invalid input, invalid UTF-8 paths, or caught Rust panics. The
* caller owns successful return values and must free them with
* audioscan_string_free. audioscan_version returns static storage and must not be
* freed.
*/

#ifndef AUDIOSCAN_H
#define AUDIOSCAN_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * JSON schema version. Bump only on a breaking change to the field set; new
 * fields are additive and do not bump it.
 */
#define SCHEMA_VERSION 1

/**
 * Analyze an audio file and return the existing audioscan JSON contract.
 *
 * Returns a newly allocated NUL-terminated JSON string on success. The caller
 * owns the returned pointer and must release it with
 * [`audioscan_string_free`]. Returns null when `path` is null, `path` is not
 * valid UTF-8, the scan configuration is invalid, analysis fails, JSON
 * serialization fails, or a Rust panic is caught.
 *
 * # Safety
 * `path` must be either null or a valid pointer to a NUL-terminated C string.
 */
char *audioscan_analyze_json(const char *path, double threshold_db, double min_gap_sec, int strict);

/**
 * Free a string returned by [`audioscan_analyze_json`].
 *
 * Passing null is a safe no-op.
 *
 * # Safety
 * `s` must be null or a pointer previously returned by
 * [`audioscan_analyze_json`] that has not already been freed.
 */
void audioscan_string_free(char *s);

/**
 * Return audioscan's static package version string.
 *
 * The returned pointer has static storage duration and must not be freed.
 */
const char *audioscan_version(void);

#endif  /* AUDIOSCAN_H */
