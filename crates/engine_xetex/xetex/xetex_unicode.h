/* tectonic/xetex_unicode.h: Pure-Rust Unicode interfaces replacing ICU.
   Copyright 2026 the Tectonic Project & FerroTeX Authors
   Licensed under the MIT License.
*/

#ifndef TECTONIC_ENGINE_XETEX_UNICODE_H
#define TECTONIC_ENGINE_XETEX_UNICODE_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifndef U_IS_BIG_ENDIAN
#if defined(__BYTE_ORDER__) && defined(__ORDER_BIG_ENDIAN__) && (__BYTE_ORDER__ == __ORDER_BIG_ENDIAN__)
#define U_IS_BIG_ENDIAN 1
#elif defined(WORDS_BIGENDIAN)
#define U_IS_BIG_ENDIAN 1
#else
#define U_IS_BIG_ENDIAN 0
#endif
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* Unicode Line Breaking */
int32_t tt_linebreak_start(const uint16_t *text, int32_t text_len, const char *locale);
int32_t tt_linebreak_next(void);
void tt_linebreak_close(void);

/* Unicode Bidirectional Algorithm */
enum {
    UBIDI_LTR = 0,
    UBIDI_RTL = 1,
    UBIDI_MIXED = 2
};

typedef struct TtBiDiContext TtBiDiContext;

TtBiDiContext* tt_bidi_create(const uint16_t *text, int32_t text_len, bool default_dir_rtl);
int32_t tt_bidi_get_direction(TtBiDiContext *ctx);
int32_t tt_bidi_count_runs(TtBiDiContext *ctx);
int32_t tt_bidi_get_visual_run(TtBiDiContext *ctx, int32_t run_index, int32_t *logical_start, int32_t *length);
void tt_bidi_destroy(TtBiDiContext *ctx);

/* Text Encodings */
typedef struct TtEncodingHandle TtEncodingHandle;

bool tt_encoding_is_supported(const char *name);
TtEncodingHandle* tt_encoding_open(const char *name);
int32_t tt_encoding_to_utf32(
    TtEncodingHandle *handle,
    const uint8_t *in_bytes,
    size_t in_len,
    uint32_t *out_utf32,
    size_t out_max_len,
    size_t *out_written
);
void tt_encoding_close(TtEncodingHandle *handle);

#ifdef __cplusplus
}
#endif

#endif /* TECTONIC_ENGINE_XETEX_UNICODE_H */
