#include <Windows.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <time.h>

DWORD  dummy_function(void* context) {
	printf("Sleep 1 second\n");
	Sleep(1000);
	return 0;
}

void create_thread_loop() {
	while (TRUE) {
		HANDLE hThread = CreateThread(
			NULL,
			0,
			&dummy_function,
			NULL,
			0,
			NULL
		);
		WaitForSingleObject(hThread, INFINITE);
		CloseHandle(hThread);
		printf("Thread 0x%x finished\n", hThread);
	}
}


void create_process_loop() {
	
}

int main()
{
	create_thread_loop();
}