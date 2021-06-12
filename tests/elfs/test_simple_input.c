#include <linux/bpf.h>


typedef unsigned char uint8_t;
typedef unsigned long int uint64_t;

uint64_t entrypoint(struct __sk_buff *skb)
{
  uint64_t x = 2;
  uint64_t y = x + skb->data;
  return y;
}
