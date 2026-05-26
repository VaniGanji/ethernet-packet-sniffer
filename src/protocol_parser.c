
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <arpa/inet.h>

#ifdef __APPLE__
#include <netinet/ip.h>
#include <netinet/tcp.h>
#include <netinet/udp.h>
#include <netinet/ip_icmp.h>
#include <net/ethernet.h>

#define IP_HDR struct ip
#define TCP_SRC th_sport
#define TCP_DST th_dport
#define UDP_SRC uh_sport
#define UDP_DST uh_dport

#else

#include <netinet/ip.h>
#include <netinet/tcp.h>
#include <netinet/udp.h>
#include <netinet/ip_icmp.h>
#include <net/ethernet.h>

#define IP_HDR struct iphdr
#define TCP_SRC source
#define TCP_DST dest
#define UDP_SRC source
#define UDP_DST dest

#endif

#include "protocol_parser.h"

static void print_mac(const uint8_t *mac)
{
    printf("%02X:%02X:%02X:%02X:%02X:%02X\n",
           mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}

static const char *ether_type_to_string(uint16_t ethertype)
{
    switch (ethertype)
    {
        case ETHERTYPE_IP:
            return "IPv4";

        case ETHERTYPE_ARP:
            return "ARP";

        case ETHERTYPE_VLAN:
            return "802.1Q VLAN";

        case ETHERTYPE_IPV6:
            return "IPv6";

        default:
            return "UNKNOWN";
    }
}

static const char *ip_protocol_to_string(uint8_t protocol)
{
    switch(protocol)
    {
        case IPPROTO_TCP:
            return "TCP";

        case IPPROTO_UDP:
            return "UDP";

        default:
            return "Not Supported";
    }
}

void parse_packet(const uint8_t *packet, uint32_t length)
{
    // Layer 2: Ethernet
    if (length < sizeof(struct ether_header))
    {
        return;
    }

    const struct ether_header *eth = (const struct ether_header *)packet;

    uint16_t ethertype = ntohs(eth->ether_type);

    printf("=============================================\n");

    printf("Ethernet Header\n");

    printf("  Src MAC : ");
    print_mac(eth->ether_shost);

    printf("  Dst MAC : ");
    print_mac(eth->ether_dhost);

    printf("  EtherType : 0x%04X (%s)\n", ethertype, ether_type_to_string(ethertype));

    uint32_t offset = sizeof(struct ether_header);

    if (ethertype == ETHERTYPE_VLAN)
    {
        if (length < offset + 4)
        {
            return;
        }

        uint16_t tci = ntohs(*(uint16_t *)(packet + offset));

        uint16_t vlan_id = tci & 0x0FFF;
        uint8_t pcp = (tci >> 13) & 0x07;

        ethertype = ntohs(*(uint16_t *)(packet + offset + 2));

        printf("VLAN Header\n");
        printf("  VLAN ID : %u\n", vlan_id);
        printf("  PCP     : %u\n", pcp);

        offset += 4;
    }

    // Only process IPv4 packets for now
    if (ethertype != ETHERTYPE_IP)
    {
        printf("  Unsupported L3 Protocol\n");
        return;
    }

    // Layer 3: IPv4
    if (length < offset + sizeof(IP_HDR))
    {
        return;
    }

    const IP_HDR *ip = (const IP_HDR *)(packet + offset);

#ifdef __APPLE__
    uint8_t protocol = ip->ip_p;
    uint32_t src_ip = ip->ip_src.s_addr;
    uint32_t dst_ip = ip->ip_dst.s_addr;
    uint8_t ihl = ip->ip_hl;
#else
    uint8_t protocol = ip->protocol;
    uint32_t src_ip = ip->saddr;
    uint32_t dst_ip = ip->daddr;
    uint8_t ihl = ip->ihl;
#endif

    char src_ip_str[INET_ADDRSTRLEN];
    char dst_ip_str[INET_ADDRSTRLEN];

    inet_ntop(AF_INET, &src_ip, src_ip_str, sizeof(src_ip_str));
    inet_ntop(AF_INET, &dst_ip, dst_ip_str, sizeof(dst_ip_str));

    printf("IPv4 Header\n");
    printf("  Src IP  : %s\n", src_ip_str);
    printf("  Dst IP  : %s\n", dst_ip_str);
    printf("  Protocol: %u (%s)\n", protocol, ip_protocol_to_string(protocol));

    offset += ihl * 4; // IHL value is measured in: 32-bit words, so multiply by 4 to get byte offset

    // Layer 4: TCP/UDP
    switch (protocol)
    {
        case IPPROTO_TCP:
        {
            if (length < offset + sizeof(struct tcphdr))
            {
                return;
            }

            const struct tcphdr *tcp = (const struct tcphdr *)(packet + offset);

            uint16_t src_port = ntohs(tcp->TCP_SRC);
            uint16_t dst_port = ntohs(tcp->TCP_DST);

            printf("TCP Header\n");
            printf("  Src Port : %u\n", src_port);
            printf("  Dst Port : %u\n", dst_port);

            /* -------- DoIP Detection -------- */
            if (src_port == 13400 || dst_port == 13400)
            {
                printf("  Automotive Protocol : DoIP\n");
            }
            break;
        }

        case IPPROTO_UDP:
        {
            if (length < offset + sizeof(struct udphdr))
            {
                return;
            }

            const struct udphdr *udp = (const struct udphdr *)(packet + offset);

            uint16_t src_port = ntohs(udp->UDP_SRC);
            uint16_t dst_port = ntohs(udp->UDP_DST);

            printf("UDP Header\n");
            printf("  Src Port : %u\n", src_port);
            printf("  Dst Port : %u\n", dst_port);

            /* -------- DoIP Detection -------- */
            if (src_port == 13400 || dst_port == 13400)
            {
                printf("  Automotive Protocol : DoIP\n");
            }

            /* -------- SOME/IP Service Discovery -------- */
            if (src_port == 30490 || dst_port == 30490)
            {
                printf("  Automotive Protocol : SOME/IP-SD\n");
            }

            break;
        }

        default:
        {
            printf("  Unsupported L4 Protocol\n");
            break;
        }
    }
}
