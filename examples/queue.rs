/// FreeRTOS queue example
/// The core concepts covered in this example include:
/// - Creating and using queues to exchange data between an ISR(Interrupt Service Routine) and a thread (both directions).
/// - Efficiently waiting for messages in the thread using queues.
///
/// This is demonstrated with a periodic timer ISR and the rust main thread, but it can be used in any thread or ISR context.
use esp_idf_hal::delay::BLOCK;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::task;
use esp_idf_hal::task::queue::Queue;
use esp_idf_hal::timer::{TimerConfig, TimerDriver};
use esp_idf_sys::{uxQueueMessagesWaiting, EspError};

fn main() -> Result<(), EspError> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_hal::sys::link_patches();

    let per = Peripherals::take()?;

    let mut timer = TimerDriver::new(per.timer00, &TimerConfig::new().auto_reload(true))?;

    // create two queue's that can hold 20/100 elements with the element size of an u8
    let queue_send = Queue::<u8>::new(20);
    let queue_recv = Queue::<u8>::new(100);

    // SAFETY: as long as queue_send/recv live we can use isr_receive & isr_send.
    let isr_recv: Queue<u8> = unsafe { Queue::new_borrowed(queue_send.as_raw()) };
    let isr_send: Queue<u8> = unsafe { Queue::new_borrowed(queue_recv.as_raw()) };

    // Every half a second
    timer.set_alarm(timer.tick_hz() / 2)?;

    // SAFTEY: make sure the timer is droped and thouse end the subscription before we drop queue_send/recv.
    // E,g don't send stuff/recv stuff from a borrowd handle after that point. When the timer object is droped it will auto unsubscribe.
    //
    // Safe since we never end main and thouse never unsubscribe nor drop queue_send/queue_recv
    unsafe {
        timer.subscribe(move || {
            // timeout value is ignored in ISR context by the API, a ISR cannot ever be allowed to block.
            if let Some((value, higher_isr_awoken)) = isr_recv.recv_front(BLOCK) {
                for i in 0..5 {
                    let send_back = value.wrapping_add(i);
                    // returns a error when queue is full
                    if isr_send.send_back(send_back, BLOCK).is_err() {
                        break;
                    }
                }

                // good practice and benifical for system performance
                if higher_isr_awoken {
                    task::do_yield();
                }
            }
        })?;
    }

    println!("Start Timer!");
    timer.enable_interrupt()?;
    timer.enable_alarm(true)?;
    timer.enable(true)?;

    let mut val = 42;
    loop {
        println!("Sending {val}");
        if queue_send.send_back(val, BLOCK).is_err() {
            println!("Qeueu is full, could not send next value to ISR");
        }

        // Waiting from the ISR to send us stuff back.
        //
        // We will go into sleep and get automatically awoken by the scheduler
        // as soon as something was added to the queue.
        match queue_recv.recv_front(BLOCK) {
            Some((value, _)) => {
                println!("Got {value} from ISR!");
            }
            None => println!("Timeout while waiting for a new message"),
        }

        // we can now check if there are still more items in the queue and read the rest.
        let unread_item_count = unsafe { uxQueueMessagesWaiting(queue_recv.as_raw()) };
        for _ in 0..unread_item_count {
            // we read exactly the number of unread items so we can unwrap and we know its not None
            let (val, _) = queue_recv.recv_front(BLOCK).unwrap();
            println!("Got additional {val} from ISR!");
        }

        val = val.wrapping_add(1);
    }
}
