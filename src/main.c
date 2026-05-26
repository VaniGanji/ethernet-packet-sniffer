#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <signal.h>
#include <pthread.h>
#include <unistd.h>
#include <pcap.h>

#include "queue.h"
#include "protocol_parser.h"

#define PACKET_SNAPLEN       2048
#define PCAP_TIMEOUT_MS      100
#define PCAP_DISPATCH_COUNT  32

static volatile int running = 1;

static packet_queue_t g_queue;

static void signal_handler(int sig)
{
    (void)sig;
    running = 0;
}

static void packet_handler(uint8_t *user, const struct pcap_pkthdr *header, const uint8_t *packet)
{
    packet_queue_t *queue = (packet_queue_t *)user;

    if (packet == NULL || header == NULL)
    {
        return;
    }

    enqueue(queue, packet, header->caplen);
}

static void *decoder_thread(void *arg)
{
    (void)arg;

    packet_t packet;

    while (running)
    {
        if (dequeue(&g_queue, &packet) == 0)
        {
            parse_packet(packet.data, packet.length);
        }
    }

    return NULL;
}

static void print_usage(const char *prog)
{
    printf("Usage: %s <interface>\n", prog);
    printf("Example: %s en0\n", prog); // eth0 for Linux, en0 for macOS
}

int main(int argc, char *argv[])
{
    char errbuf[PCAP_ERRBUF_SIZE];

    pcap_t *handle;

    pthread_t decoder_tid;

    int ret;

    if (argc != 2)
    {
        print_usage(argv[0]);
        return EXIT_FAILURE;
    }

    const char *interface = argv[1];

    signal(SIGINT, signal_handler);

    queue_init(&g_queue);

    // Open the specified network interface for packet capture
    handle = pcap_open_live(interface, PACKET_SNAPLEN, 1, PCAP_TIMEOUT_MS, errbuf);

    if (handle == NULL)
    {
        fprintf(stderr, "ERROR: pcap_open_live failed: %s\n", errbuf);
        return EXIT_FAILURE;
    }

    // Ensure the capture device is Ethernet
    if (pcap_datalink(handle) != DLT_EN10MB)
    {
        fprintf(stderr, "ERROR: Only Ethernet interfaces are supported\n");

        pcap_close(handle);
        return EXIT_FAILURE;
    }

    ret = pthread_create(&decoder_tid, NULL, decoder_thread, NULL);

    if (ret != 0)
    {
        fprintf(stderr, "ERROR: Failed to create decoder thread\n");

        pcap_close(handle);
        return EXIT_FAILURE;
    }

    printf("=============================================\n");
    printf(" Ethernet Packet Sniffer Started\n");
    printf("=============================================\n");
    printf(" Interface : %s\n", interface);
    printf(" Snaplen   : %d bytes\n", PACKET_SNAPLEN);
    printf(" Timeout   : %d ms\n", PCAP_TIMEOUT_MS);
    printf("=============================================\n");

    while (running)
    {
        ret = pcap_dispatch(handle, PCAP_DISPATCH_COUNT, packet_handler, (uint8_t *)&g_queue);

        if (ret < 0)
        {
            fprintf(stderr, "ERROR: pcap_dispatch failed: %s\n", pcap_geterr(handle));

            break;
        }

        if (ret == 0)
        {
            usleep(1000);
        }
    }

    printf("\nStopping capture...\n");

    pcap_breakloop(handle);

    pthread_cancel(decoder_tid);

    pthread_join(decoder_tid, NULL);

    pcap_close(handle);

    printf("Sniffer stopped successfully\n");

    return EXIT_SUCCESS;
}
