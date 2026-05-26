#include <stdio.h>
#include <string.h>

#include "queue.h"

void queue_init(packet_queue_t *queue)
{
    queue->head = 0;
    queue->tail = 0;
    queue->count = 0;

    pthread_mutex_init(&queue->mutex, NULL);
    pthread_cond_init(&queue->not_empty, NULL);
    pthread_cond_init(&queue->not_full, NULL);
}

void enqueue(packet_queue_t *queue, const uint8_t *data, uint32_t length)
{
    pthread_mutex_lock(&queue->mutex);

    while (queue->count == QUEUE_SIZE)
    {
        pthread_cond_wait(&queue->not_full, &queue->mutex);
    }

    if (length > MAX_PACKET_SIZE)
    {
        length = MAX_PACKET_SIZE;
    }

    memcpy(queue->packets[queue->tail].data, data, length);

    queue->packets[queue->tail].length = length;

    queue->tail = (queue->tail + 1) % QUEUE_SIZE;
    queue->count++;

    pthread_cond_signal(&queue->not_empty);

    pthread_mutex_unlock(&queue->mutex);
}

int dequeue(packet_queue_t *queue, packet_t *packet)
{
    pthread_mutex_lock(&queue->mutex);

    while (queue->count == 0)
    {
        pthread_cond_wait(&queue->not_empty, &queue->mutex);
    }

    memcpy(packet, &queue->packets[queue->head], sizeof(packet_t));

    queue->head = (queue->head + 1) % QUEUE_SIZE;
    queue->count--;

    pthread_cond_signal(&queue->not_full);

    pthread_mutex_unlock(&queue->mutex);

    return 0;
}
