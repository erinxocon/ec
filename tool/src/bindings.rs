use std::{io};

use pyo3::prelude::*;
use pyo3::exceptions::PyRuntimeError;
use pyo3::types::PyType;

use hidapi::HidApi;

use crate::{Ec, Error, Access, AccessHid};

fn to_py_err(err: Error) -> PyErr {
    PyRuntimeError::new_err(format!("EC error: {err:?}"))
}

fn to_py_hid(err: hidapi::HidError) -> PyErr {
    PyRuntimeError::new_err(format!("hidapi: {err}"))
}

fn ec_board(ec: &mut Ec<Box<dyn Access>>) -> Result<String, Error> {
    let data_size = unsafe { ec.access().data_size() };
    let mut data = vec![0; data_size];
    let size = unsafe { ec.board(&mut data)? };
    data.truncate(size);
    String::from_utf8(data).map_err(|err| Error::Io(io::Error::new(io::ErrorKind::Other, err)))
}

fn ec_version(ec: &mut Ec<Box<dyn Access>>) -> Result<String, Error> {
    let data_size = unsafe { ec.access().data_size() };
    let mut data = vec![0; data_size];
    let size = unsafe { ec.version(&mut data)? };
    data.truncate(size);
    String::from_utf8(data).map_err(|err| Error::Io(io::Error::new(io::ErrorKind::Other, err)))
}

#[pyclass]
#[derive(Clone)]
pub struct Led {
    #[pyo3(get)]
    index: u8,
    #[pyo3(get)]
    color: (u8, u8, u8),
    sync_color: Option<(u8, u8, u8)>,
}

#[pymethods]
impl Led {
    #[new]
    pub fn new(index: u8, r: u8, g: u8, b: u8) -> Self {
        Self {index, color: (r, g, b), sync_color: None}
    }

    #[classmethod]
    pub fn from_rgb(_cls: Bound<'_, PyType>, index: u8, color: (u8, u8, u8)) -> Self {
        Self { index, color, sync_color: None}
    }

    #[classmethod]
    pub fn from_hex(_cls: Bound<'_, PyType>, index: u8, hex: u32) -> Self {
        let r = ((hex >> 16) & 0xFF) as u8;
        let g = ((hex >> 8) & 0xFF) as u8;
        let b = (hex & 0xFF) as u8;
        Self {
            index,
            color: (r, g, b),
            sync_color: None,
        }
    }

    pub fn set_color_rgb(&mut self, r: u8, g:u8, b: u8) -> PyResult<()> {
        self.color = (r, g, b);
        Ok(())
    }

    pub fn set_color_hex(&mut self, hex: u32) -> PyResult<()> {
        self.color = (
            ((hex >> 16) & 0xFF) as u8,
            ((hex >> 8) & 0xFF) as u8,
            (hex & 0xFF) as u8,
        );
        Ok(())
    }
}

impl Led {
    fn sync(&mut self, ec: &mut Ec<Box<dyn Access>>) -> Result<(), Error> {
        if self.sync_color != Some(self.color) {
            let (r, g, b) = self.color;
            unsafe {
                ec.led_set_color(self.index, r, g, b)?;
            }
            self.sync_color = Some(self.color);
        }
        Ok(())
    }
}

#[pyclass]
#[derive(Clone)]
pub struct FrameBuffer {
    #[pyo3(get)]
    leds: Vec<Vec<Led>>,
    #[pyo3(get)]
    width: u8,
    #[pyo3(get)]
    height: u8,
    #[pyo3(get)]
    num_leds: usize,
}

#[pymethods]
impl FrameBuffer {
    #[new]
    pub fn new(led_map: Vec<Vec<u8>>) -> Self {
        let leds: Vec<Vec<Led>> = led_map
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .filter(|&idx| idx != 255)
                    .map(|idx| Led {
                        index: idx,
                        color: (0, 0, 0),
                        sync_color: None,
                    })
                    .collect()
            })
            .collect();

        let height = leds.len() as u8;
        let width = leds.iter().map(|row| row.len()).max().unwrap_or(0) as u8;
        let num_leds = leds.iter().map(|row| row.len()).sum::<usize>();
        Self { leds, width, height, num_leds }
    }

    pub fn get(&self, row: usize, col: usize) -> PyResult<Option<Led>> {
        Ok(self.leds.get(row).and_then(|r| r.get(col)).cloned())
    }

    pub fn set(&mut self, row: usize, col: usize, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.leds
            .get_mut(row)
            .and_then(|row_vec| row_vec.get_mut(col))
            .map(|led| led.color = (r, g, b));
            Ok(())
    }

    pub fn fill(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        for row in &mut self.leds {
            for led in row.iter_mut() {
                led.color = (r, g, b);
            }
        }
        Ok(())
    }

    pub fn clear(&mut self) -> PyResult<()> {
        self.fill(0, 0, 0)
    }

    #[getter]
    fn flat_leds(&self) -> Vec<Led> {
        self.leds.iter().flatten().cloned().collect()
    }
}

impl FrameBuffer {
    fn render(&mut self, ec: &mut Ec<Box<dyn Access>>) -> Result<(), Error> {
        for row in &mut self.leds {
            for led in row {
                led.sync(ec)?;
            }
        }
        Ok(())
    }
}

#[pyclass(unsendable)]
pub struct EcController {
    ec: Ec<Box<dyn Access>>,
    #[pyo3(get)]
    led_map: Vec<Vec<u8>>,
    #[pyo3(get)]
    framebuffer: FrameBuffer,
    saved_layer_mode: Option<(u8, u8)>,
}

#[pymethods]
impl EcController {
    #[new]
    pub fn new() -> PyResult<Self> {
        let ni = 255;
        let api = HidApi::new().map_err(to_py_hid)?;
        for info in api.device_list() {
            match (info.vendor_id(), info.product_id(), info.interface_number()) {
                // System76 Launch keyboards
                (0x3384, 0x0001..=0x000A, 1) => {
                    let device = info.open_device(&api).map_err(to_py_hid)?;
                    let access = AccessHid::new(device, 10, 100).map_err(to_py_err)?;
                    let ec = unsafe { Ec::new(access).map_err(to_py_err)? }.into_dyn();

                    //refactor this to set these per keyboard layout based on device info
                    let led_map = vec![
                        vec![69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83],
                        vec![68, 67, 66, 65, 64, 63, 62, 61, 60, 59, 58, 57, 56, 55, 54],
                        vec![39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53],
                        vec![38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27, 26, ni, 25],
                        vec![12, 11, 10,  9,  8,  7,  6,  5,  4,  3,  2,  1,  0, ni, ni],
                        vec![13, 14, 15, 16, 17, ni, 18, 19, 20, 21, ni, 22, 23, 24, ni],
                    ];
                    let framebuffer = FrameBuffer::new(led_map.clone());
                    return Ok(Self { ec, led_map, framebuffer, saved_layer_mode: None });
                }
                _ => {}
            }
        }

        Err(PyRuntimeError::new_err("No compatible EC HID device found"))
    }

    pub fn open(&mut self) -> PyResult<()> {
        // let (mode, speed) = unsafe { self.ec.led_get_mode(1).map_err(to_py_err)? };
        // self.saved_layer_mode = Some((mode, speed));
        for layer in 0..4 {
            println!("Set layer {} mode: {:?}", layer, unsafe {
                self.ec.led_set_mode(layer, 1, 0)
            });
            println!("Set layer {} brightness: {:?}", layer, unsafe {
                self.ec.led_set_value(0xF0 | layer, 0xFF)
            });
        }
        Ok(())
    }

    pub fn close(&mut self) -> PyResult<()> {
        // if let Some((mode, speed)) = self.saved_layer_mode.take() {
            // unsafe {
            //     self.ec.led_set_mode(1, mode, speed).map_err(to_py_err)?;
            // }
        // }
        Ok(())
    }

    #[getter]
    fn board(&mut self) -> PyResult<String> {
        ec_board(&mut self.ec).map_err(to_py_err)
    }

    #[getter]
    fn version(&mut self) -> PyResult<String> {
        ec_version(&mut self.ec).map_err(to_py_err)
    }

    pub fn led_get_value(&mut self, index: u8) -> PyResult<(u8, u8)> {
        unsafe { self.ec.led_get_value(index).map_err(to_py_err) }
    }

    pub fn led_set_value(&mut self, index: u8, value: u8) -> PyResult<()> {
        unsafe { self.ec.led_set_value(index, value).map_err(to_py_err) }
    }

    pub fn led_get_mode(&mut self, layer: u8) -> PyResult<(u8, u8)> {
        unsafe { self.ec.led_get_mode(layer).map_err(to_py_err) }
    }

    pub fn led_set_mode(&mut self, layer: u8, mode: u8, speed: u8) -> PyResult<()> {
        unsafe { self.ec.led_set_mode(layer, mode, speed).map_err(to_py_err) }
    }

    pub fn led_get_color(&mut self, index: u8) -> PyResult<(u8, u8, u8)> {
        unsafe { self.ec.led_get_color(index).map_err(to_py_err) }
    }

    pub fn led_set_color(&mut self, index: u8, r: u8, g: u8, b: u8) -> PyResult<()> {
        unsafe { self.ec.led_set_color(index, r, g, b).map_err(to_py_err) }
    }

    pub fn get_led(&mut self, index: u8) -> PyResult<Led> {
        let (r, g, b) = unsafe { self.ec.led_get_color(index).map_err(to_py_err)? };
        Ok(Led {
            index,
            color: (r, g, b),
            sync_color: None,
        })
    }

    pub fn set_led(&mut self, mut led: Led) -> PyResult<()> {
        led.sync(&mut self.ec).map_err(to_py_err)?;
        Ok(())
    }

    pub fn render_framebuffer(&mut self) -> PyResult<()> {
        self.framebuffer.render(&mut self.ec).map_err(to_py_err)?;
        Ok(())
    }
}

