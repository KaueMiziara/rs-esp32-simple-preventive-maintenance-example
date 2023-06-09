#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use hal::{
    clock::ClockControl, i2c, peripherals::Peripherals, prelude::*, timer::TimerGroup, Delay, Rtc,
    IO,
};
use mpu6050::*;

// Compile, flash and run:
// source ~/export-esp.sh
// cargo espflash --release --monitor

enum Limit {
    Mechanical,
    Temperature,
}

const MECHANICAL_LIMIT: f32 = 0.8;
const TEMPERATURE_LIMIT: f32 = 2.5;

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let mut system = peripherals.DPORT.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    // Disable the RTC and TIMG watchdog timers
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt1 = timer_group1.wdt;
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();

    // Initialize Delay
    let mut delay = Delay::new(&clocks);

    // Initialize IO && Pin definitions
    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let (mut internal_led, mut buzzer, sda, scl) = (
        io.pins.gpio2.into_push_pull_output(),
        io.pins.gpio33.into_push_pull_output(),
        io.pins.gpio21,
        io.pins.gpio22,
    );

    // Configure I2C
    let i2c = i2c::I2C::new(
        peripherals.I2C0,
        sda,
        scl,
        100u32.kHz(),
        &mut system.peripheral_clock_control,
        &clocks,
    );
    delay.delay_ms(255u8);

    // Initialize MPU6050 module
    let mut mpu = Mpu6050::new(i2c);
    mpu.init(&mut delay)
        .expect("Error while initializing MPU6050");

    // Define reference values
    let mut acc_ref = mpu.get_acc();
    let temp_ref = mpu.get_temp();

    // Only sudden moves should activate the buzzer.
    // For that, each loop cycle should reset the accelerometer's reference.
    // Otherwise, changing the MPU's position would also sound the alarm.
    let mut reset_reference = true;

    println!("---");
    loop {
        if reset_reference {
            acc_ref = mpu.get_acc();

            reset_reference = false;
            delay.delay_ms(100u8);
        } else {
            // Update values
            let acc = mpu.get_acc();
            let gyro = mpu.get_gyro();
            let temp = mpu.get_temp();
            // All of those "get" methods return a Result<T,E>.
            // "acc" and "gyro"'s 'T' is equivalent to an array of 3 f32, [x, y, z];
            // "temp"'s T is an f32

            // Accelerometer data
            match acc {
                Ok(data) => {
                    println!("Accelerometer:");
                    println!("Ax: {} m/s^2", data[0]);
                    println!("Ay: {} m/s^2", data[1]);
                    println!("Az: {} m/s^2", data[2]);

                    let acc_ref_x = acc_ref.as_ref().unwrap()[0];

                    let mut delta = data[0] - acc_ref_x;

                    if delta.abs() >= MECHANICAL_LIMIT {
                        println!("MECHANICAL STRESS DETECTED!");
                        println!("Current: {}", data[0]);
                        println!("Reference: {}", acc_ref_x);
                        println!("Delta: {}", delta);

                        alarm(
                            &mut buzzer,
                            &mut internal_led,
                            &Limit::Mechanical,
                            &mut delay,
                        );
                    }
                }
                Err(_) => panic!("Error reading data from the accelerometer"),
            };

            // Gyroscope data
            match gyro {
                Ok(data) => {
                    println!("Gyroscope:");
                    println!("Gx: {} rad/s", data[0]);
                    println!("Gy: {} rad/s", data[1]);
                    println!("Gz: {} rad/s", data[2]);
                }
                Err(_) => panic!("Error reading data from the gyroscope"),
            };

            // Temperature data
            match temp {
                Ok(data) => {
                    println!("Temperature:\n{} ºC", data);

                    let temp_ref = temp_ref.as_ref().unwrap();
                    let mut delta = data - temp_ref;

                    if delta.abs() >= TEMPERATURE_LIMIT {
                        println!("OVERHEATING DETECTED");
                        println!("Current: {}", data);
                        println!("Reference: {}", temp_ref);
                        println!("Delta: {}", delta);

                        alarm(
                            &mut buzzer,
                            &mut internal_led,
                            &Limit::Temperature,
                            &mut delay,
                        );
                    }
                }
                Err(_) => panic!("Error reading data from the temperature sensor"),
            }

            println!("---");

            reset_reference = true;
            delay.delay_ms(500u16);
        }
    }
}

// Functions to make the alarm sound
fn alarm(
    buzzer: &mut hal::gpio::GpioPin<
        hal::gpio::Output<hal::gpio::PushPull>,
        hal::gpio::Bank1GpioRegisterAccess,
        hal::gpio::DualCoreInteruptStatusRegisterAccessBank1,
        hal::gpio::InputOutputAnalogPinType,
        hal::gpio::Gpio33Signals,
        33,
    >,
    led: &mut hal::gpio::GpioPin<
        hal::gpio::Output<hal::gpio::PushPull>,
        hal::gpio::Bank0GpioRegisterAccess,
        hal::gpio::DualCoreInteruptStatusRegisterAccessBank0,
        hal::gpio::InputOutputAnalogPinType,
        hal::gpio::Gpio2Signals,
        2,
    >,
    limit: &Limit,
    delay: &mut Delay,
) {
    let buzzes: u8 = match limit {
        Limit::Mechanical => 3,
        Limit::Temperature => 9,
    };

    for _ in 0..buzzes {
        buzzer.set_high().unwrap();
        led.set_high().unwrap();

        alarm_time(limit, delay);

        buzzer.set_low().unwrap();
        led.set_low().unwrap();

        alarm_time(limit, delay);
    }
}

fn alarm_time(limit: &Limit, delay: &mut Delay) {
    match limit {
        Limit::Mechanical => delay.delay_ms(100u8),
        Limit::Temperature => delay.delay_ms(50u8),
    }
}

// abs() method for f32 is not defined outside std
pub trait Absolute {
    fn abs(&mut self) -> Self;
}

impl Absolute for f32 {
    fn abs(&mut self) -> Self {
        if self.is_sign_negative() {
            *self *= -1.0;
        }
        *self
    }
}
