/* tectonic/xetex-core.h: core XeTeX types and #includes.
   Copyright 2016-2020 the Tectonic Project
   Licensed under the MIT License.
*/

#ifndef TECTONIC_XETEX_CORE_H
#define TECTONIC_XETEX_CORE_H

#include "tectonic_bridge_core.h"

/* Pure-Rust Unicode interfaces */
#include "xetex_unicode.h"

/* fontconfig */
#ifndef XETEX_MAC
#include <fontconfig/fontconfig.h>
#endif

#include <string.h>

/* HarfBuzz-compatible tag and feature types used by layout engine */
#ifndef HB_TAG_NONE
typedef uint32_t hb_tag_t;
#define HB_TAG_NONE 0x00000000u

typedef struct hb_feature_t {
    hb_tag_t tag;
    uint32_t value;
    unsigned int start;
    unsigned int end;
} hb_feature_t;
#endif

static inline hb_tag_t hb_tag_from_string(const char *str, int len) {
    char tag[4] = {' ', ' ', ' ', ' '};
    if (len < 0) {
        len = (int) strlen(str);
    }
    for (int i = 0; i < 4 && i < len; i++) {
        tag[i] = str[i];
    }
    return (((uint32_t)(uint8_t)(tag[0]) << 24) |
            ((uint32_t)(uint8_t)(tag[1]) << 16) |
            ((uint32_t)(uint8_t)(tag[2]) << 8)  |
            ((uint32_t)(uint8_t)(tag[3])));
}

/* Endianness foo */

#ifdef WORDS_BIGENDIAN
#define US_NATIVE_UTF16 UTF16BE
#define UTF16_NATIVE kForm_UTF16BE
#define NATIVE_UTF32 kForm_UTF32BE
#define UCNV_UTF32_NativeEndian UCNV_UTF32_BigEndian
#else
#define US_NATIVE_UTF16 UTF16LE
#define UTF16_NATIVE kForm_UTF16LE
#define NATIVE_UTF32 kForm_UTF32LE
#define UCNV_UTF32_NativeEndian UCNV_UTF32_LittleEndian
#endif

/* our typedefs */

typedef int32_t scaled_t;

typedef enum {
    SELECTOR_FILE_0 = 0,
    SELECTOR_FILE_15 = 15,
    SELECTOR_NO_PRINT = 16,
    SELECTOR_TERM_ONLY = 17,
    SELECTOR_LOG_ONLY = 18,
    SELECTOR_TERM_AND_LOG = 19,
    SELECTOR_PSEUDO = 20,
    SELECTOR_NEW_STRING = 21
} selector_t;

/* Misc */

#define FONT_FLAGS_COLORED  0x01
#define FONT_FLAGS_VERTICAL 0x02

#endif /* not TECTONIC_XETEX_CORE_H */
