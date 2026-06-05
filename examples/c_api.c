/*
 * Build:
 * cargo build --features capi && cc examples/c_api.c -Iinclude -Ltarget/debug -laudioscan -o /tmp/c_api
 */

#include <stdio.h>

#include "audioscan.h"

int main(int argc, char **argv) {
    char *json;

    if (argc != 2) {
        fprintf(stderr, "usage: %s PATH\n", argv[0]);
        return 2;
    }

    json = audioscan_analyze_json(argv[1], -30.0, 5.0, 0);
    if (json == NULL) {
        fprintf(stderr, "audioscan analysis failed\n");
        return 1;
    }

    puts(json);
    audioscan_string_free(json);
    return 0;
}
