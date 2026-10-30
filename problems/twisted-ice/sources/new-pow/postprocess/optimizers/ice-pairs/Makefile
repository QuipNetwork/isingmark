CC ?= gcc
CFLAGS ?= -O3 -march=native -shared -fPIC -Wall -Wextra

all: libgreedy.so libpairworm.so

libgreedy.so: greedy.c
	$(CC) $(CFLAGS) -o $@ $<

libpairworm.so: pairworm.c
	$(CC) $(CFLAGS) -o $@ $< -lm

clean:
	rm -f libgreedy.so libpairworm.so

.PHONY: all clean
