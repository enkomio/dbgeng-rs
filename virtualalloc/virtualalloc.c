#include <Windows.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <time.h>

uint8_t shellcode[] = { 
	0x48, 0x89, 0xC8, 	// mov rax,rcx
	0x48, 0xFF, 0xC0, 	// inc rax
	0xC3 				// ret
};

void main() {
	uint32_t i = 0;
	srand(time(NULL));
	while(1) {		
		int size = (rand() % 0x1000000) + sizeof(shellcode) * 2;
	
		PVOID mem = VirtualAlloc(
			NULL,
			size,
			MEM_COMMIT,
			PAGE_EXECUTE_READWRITE
		);
		if (!mem) {
			return;
		}
		printf("Allocated 0x%x bytes of executable memory at: 0x%llx\n", size, mem);
		Sleep(1000);
		
		memcpy(mem, shellcode, sizeof(shellcode));
		i = ((int (*)(int))mem)(i);
		printf("Allocated shellcode execution results in value: 0x%llx\n", i);
		Sleep(1000);
		
		VirtualFree(
			mem,
			0,
			MEM_RELEASE
		);
		printf("Free memory 0x%llx\n", mem);
		Sleep(1000);	
	}	
}