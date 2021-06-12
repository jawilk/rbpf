typedef unsigned char uint8_t;
typedef unsigned long int uint64_t;


uint64_t __attribute__ ((noinline)) add_1(uint64_t x);
uint64_t __attribute__ ((noinline)) add_2(uint64_t x);

uint64_t entrypoint(const uint8_t *input_1)
{
  //uint64_t array[64];
  uint64_t x = 2;
  //syscall_2(&array[2]);
  uint64_t y = add_1(x);
  uint64_t z = y+1;
  uint64_t a = add_2(z);
  return a;
}

uint64_t __attribute__ ((noinline)) add_1(uint64_t i) {
  return i+1;
}

uint64_t __attribute__ ((noinline)) add_2(uint64_t i) {
  return i+1;
}
