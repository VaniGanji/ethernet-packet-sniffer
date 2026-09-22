#ifndef PROTOCOL_PARSER_H
#define PROTOCOL_PARSER_H

#include <stdint.h>

void parse_packet(const uint8_t *packet, uint32_t length);

#endif
