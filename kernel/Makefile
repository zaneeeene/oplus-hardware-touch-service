# Kernel module Makefile for xiaomi-touch HAL driver

obj-$(CONFIG_TOUCHSCREEN_XIAOMI_TOUCH_HAL) += xiaomi_touch_hal.o

xiaomi_touch_hal-y := xiaomi_touch_hal.o

ccflags-y += -I$(src)/include

# For building out-of-tree
ifneq ($(KERNELRELEASE),)
# Building from kernel build system
else
KERNELDIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)

default:
	$(MAKE) -C $(KERNELDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KERNELDIR) M=$(PWD) clean
endif
