use std::{io};

use pyo3::prelude::*;
use pyo3::exceptions::PyRuntimeError;

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

#[pyclass(unsendable)]
pub struct EcController {
    ec: Ec<Box<dyn Access>>,
    led_map: Vec<Vec<u8>>,
}

#[pymethods]
impl EcController {
    #[new]
    pub fn new() -> PyResult<Self> {
        let ni = 255; // use a value out of range for your keyboard as a sentinel
        let led_map = vec![
            vec![69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83],
            vec![68, 67, 66, 65, 64, 63, 62, 61, 60, 59, 58, 57, 56, 55, 54],
            vec![39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53],
            vec![38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27, 26, ni, 25],
            vec![12, 11, 10,  9,  8,  7,  6,  5,  4,  3,  2,  1,  0, ni, ni],
            vec![13, 14, 15, 16, 17, ni, 18, 19, 20, 21, ni, 22, 23, 24, ni],
        ];
        let api = HidApi::new().map_err(to_py_hid)?;
        for info in api.device_list() {
            match (info.vendor_id(), info.product_id(), info.interface_number()) {
                // System76 Launch keyboards
                (0x3384, 0x0001..=0x000A, 1) => {
                    let device = info.open_device(&api).map_err(to_py_hid)?;
                    let access = AccessHid::new(device, 10, 100).map_err(to_py_err)?;
                    let ec = unsafe { Ec::new(access).map_err(to_py_err)? }.into_dyn();
                    return Ok(Self { ec, led_map });
                }
                _ => {}
            }
        }

        Err(PyRuntimeError::new_err("No compatible EC HID device found"))
    }

    #[getter]
    fn led_map(&self) -> Vec<Vec<u8>> {
        self.led_map.clone()
    }

    #[getter]
    fn board(&mut self) -> PyResult<String> {
        ec_board(&mut self.ec).map_err(to_py_err)
    }

    #[getter]
    fn version(&mut self) -> PyResult<String> {
        ec_version(&mut self.ec).map_err(to_py_err)
    }

    pub fn test_led_layers(&mut self) -> PyResult<()> {
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
}

