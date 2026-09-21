//! Queue set example
//!
//! Demonstrates how to use [`QueueSet3`] to block on three queues of different
//! item types simultaneously.
//!
//! Three producer threads send items to separate queues:
//!   - Thread A sends `u32` sensor readings every second.
//!   - Thread B sends `f32` temperature values every 1.5 seconds.
//!   - Thread C sends `i16` pressure deltas every 2 seconds.
//!
//! The main task uses `QueueSet3` to receive from whichever queue has an item
//! ready first, without polling or dedicated per-queue receive loops.

use std::sync::Arc;
use std::time::Duration;

use esp_idf_hal::task::queue::{Queue, QueueSet3, QueueSet3Selected};

fn main() -> anyhow::Result<()> {
    esp_idf_hal::sys::link_patches();

    println!("Queue set example started");

    // Create three queues with a capacity of 4 items each.
    // Wrap them in Arc so they can be shared with producer threads.
    let q_sensor: Arc<Queue<u32>> = Arc::new(Queue::new(4));
    let q_temp: Arc<Queue<f32>> = Arc::new(Queue::new(4));
    let q_pressure: Arc<Queue<i16>> = Arc::new(Queue::new(4));

    // Clone Arc handles for the producer threads.
    let q_sensor_tx = q_sensor.clone();
    let q_temp_tx = q_temp.clone();
    let q_pressure_tx = q_pressure.clone();

    // Producer A: sends u32 sensor readings.
    std::thread::Builder::new()
        .stack_size(4096)
        .spawn(move || {
            for reading in 0u32..=u32::MAX {
                println!("[producer A] sensor reading: {reading}");
                q_sensor_tx
                    .send_back(reading, esp_idf_hal::delay::BLOCK)
                    .unwrap();
                std::thread::sleep(Duration::from_millis(1000));
            }
        })?;

    // Producer B: sends f32 temperature values.
    std::thread::Builder::new()
        .stack_size(4096)
        .spawn(move || {
            let mut temp = 20.0f32;
            loop {
                println!("[producer B] temperature: {temp:.1}");
                q_temp_tx
                    .send_back(temp, esp_idf_hal::delay::BLOCK)
                    .unwrap();
                temp += 0.5;
                std::thread::sleep(Duration::from_millis(1500));
            }
        })?;

    // Producer C: sends i16 pressure deltas.
    std::thread::Builder::new()
        .stack_size(4096)
        .spawn(move || {
            let mut delta = 0i16;
            loop {
                println!("[producer C] pressure delta: {delta}");
                q_pressure_tx
                    .send_back(delta, esp_idf_hal::delay::BLOCK)
                    .unwrap();
                delta += 1;
                std::thread::sleep(Duration::from_millis(2000));
            }
        })?;

    // Build the queue set.  All queues must be empty here, which is required
    // by FreeRTOS before adding them to a set.
    let qs = QueueSet3::new(&*q_sensor, &*q_temp, &*q_pressure)?;

    println!("Listening on queue set …");

    loop {
        // Block until any queue has an item; BLOCK = wait forever.
        match qs.select_from_set(esp_idf_hal::delay::BLOCK) {
            Some(QueueSet3Selected::Q0(v)) => {
                println!("[consumer] sensor reading:  {v}");
            }
            Some(QueueSet3Selected::Q1(v)) => {
                println!("[consumer] temperature:     {v:.1}");
            }
            Some(QueueSet3Selected::Q2(v)) => {
                println!("[consumer] pressure delta:  {v}");
            }
            None => {
                // Timeout — won't happen with BLOCK, but handle it anyway.
                println!("[consumer] select_from_set timed out");
            }
        }
    }
}
