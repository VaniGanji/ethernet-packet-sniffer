#ifndef QUEUE_H
#define QUEUE_H

#include <stdint.h>
#include <pthread.h>

#define MAX_PACKET_SIZE 2048
#define QUEUE_SIZE 1024

typedef struct
{
    uint8_t data[MAX_PACKET_SIZE];
    uint32_t length;
} packet_t;

typedef struct
{
    packet_t packets[QUEUE_SIZE];

    int head;
    int tail;
    int count;

    pthread_mutex_t mutex;
    pthread_cond_t not_empty;
    pthread_cond_t not_full;

} packet_queue_t;

void queue_init(packet_queue_t *queue);
void enqueue(packet_queue_t *queue, const uint8_t *data, uint32_t length);
int dequeue(packet_queue_t *queue, packet_t *packet);

#endif
