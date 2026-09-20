//! Pure-Rust port of `xetex-stringpool.c` — TeX's flat UTF-16 string pool.
//!
//! Strings are stored as consecutive code units in `str_pool`; `str_start`
//! records each string's first index and is addressed by
//! `str_number - TOO_BIG_CHAR`. String numbers below `TOO_BIG_CHAR` are bare
//! character codes; numbers at or above it index the pool. `pool_ptr` is the
//! write cursor (kept equal to `str_pool.len()`), and `str_ptr` counts from
//! `TOO_BIG_CHAR` upward.

use crate::state::{TeXEngineState, TeXMemoryError, EMPTY_STRING, TOO_BIG_CHAR};

/// Pool constants loaded by `get_strings_started`, verbatim from C's
/// `string_constants` table. The first entry guards code that relies on
/// magic string-pool addresses; the second is `EMPTY_STRING`.
const STRING_CONSTANTS: [&str; 2] = ["this marks the start of the stringpool", ""];

impl TeXEngineState {
    /// Appends one code unit at `pool_ptr`, preserving the
    /// `pool_ptr == str_pool.len()` invariant while tolerating rewound
    /// cursors (C writes into a preallocated buffer).
    pub(crate) fn pool_push(&mut self, c: i32) {
        if self.pool_ptr as usize >= self.str_pool.len() {
            self.str_pool.push(c);
        } else {
            self.str_pool[self.pool_ptr as usize] = c;
        }
        self.pool_ptr += 1;
    }

    /// Display length of a `str_number` (`length`): pool strings use their
    /// stored extent; bare character codes use the `^^` escape conventions.
    pub fn length(&self, s: i32) -> i32 {
        if s >= TOO_BIG_CHAR {
            let idx = (s - TOO_BIG_CHAR) as usize;
            return self.str_start.get(idx + 1).copied().unwrap_or(0)
                - self.str_start.get(idx).copied().unwrap_or(0);
        }
        if (32..127).contains(&s) {
            1
        } else if s <= 127 {
            3
        } else if s < 256 {
            4
        } else {
            8
        }
    }

    /// Seals the text accumulated since the previous string and returns its
    /// `str_number` (`make_string`).
    pub fn make_string(&mut self) -> Result<i32, TeXMemoryError> {
        if self.str_ptr == self.max_strings {
            return Err(TeXMemoryError::OutOfMemory(
                (self.max_strings - self.init_str_ptr) as usize,
            ));
        }
        self.str_ptr += 1;
        let idx = (self.str_ptr - TOO_BIG_CHAR) as usize;
        if self.str_start.len() <= idx {
            self.str_start.resize(idx + 1, 0);
        }
        self.str_start[idx] = self.pool_ptr;
        Ok(self.str_ptr - 1)
    }

    /// Copies pool string `s` onto the pool tail (`append_str`).
    pub fn append_str(&mut self, s: i32) -> Result<(), TeXMemoryError> {
        let i = self.length(s);
        if self.pool_ptr + i > self.pool_size {
            return Err(TeXMemoryError::OutOfMemory(
                (self.pool_size - self.init_pool_ptr) as usize,
            ));
        }
        let idx = (s - TOO_BIG_CHAR) as usize;
        for j in (self.str_start[idx] as usize..).take(i as usize) {
            let unit = self.str_pool[j];
            self.pool_push(unit);
        }
        Ok(())
    }

    /// Compares pool string `s` against `buffer[k..]` (`str_eq_buf`), mapping
    /// astral buffer scalars to surrogate pairs.
    pub fn str_eq_buf(&self, s: i32, k: usize) -> bool {
        let idx = (s - TOO_BIG_CHAR) as usize;
        let end = self.str_start[idx + 1] as usize;
        let mut j = self.str_start[idx] as usize;
        let mut k = k;
        while j < end {
            let b = self.buffer.get(k).copied().unwrap_or(0);
            if b >= 65536 {
                if self.str_pool[j] != 55296 + (b - 65536) / 1024
                    || self.str_pool.get(j + 1).copied().unwrap_or(0) != 56320 + (b - 65536) % 1024
                {
                    return false;
                }
                j += 1;
            } else if self.str_pool[j] != b {
                return false;
            }
            j += 1;
            k += 1;
        }
        true
    }

    /// Compares two `str_number`s for equal text (`str_eq_str`); either side
    /// may be a bare character code.
    pub fn str_eq_str(&self, s: i32, t: i32) -> bool {
        if self.length(s) != self.length(t) {
            return false;
        }
        if self.length(s) == 1 {
            if s < 65536 {
                if t < 65536 {
                    s == t
                } else {
                    s == self.str_pool[self.str_start[(t - 65536) as usize] as usize]
                }
            } else if t < 65536 {
                self.str_pool[self.str_start[(s - 65536) as usize] as usize] == t
            } else {
                self.str_pool[self.str_start[(s - 65536) as usize] as usize]
                    == self.str_pool[self.str_start[(t - 65536) as usize] as usize]
            }
        } else {
            let mut j = self.str_start[(s - 65536) as usize] as usize;
            let mut k = self.str_start[(t - 65536) as usize] as usize;
            let end = self.str_start[(s + 1 - 65536) as usize] as usize;
            while j < end {
                if self.str_pool[j] != self.str_pool[k] {
                    return false;
                }
                j += 1;
                k += 1;
            }
            true
        }
    }

    /// Finds an existing pool string equal to `search`, scanning downward
    /// from `search - 1` (`search_string`); returns `EMPTY_STRING` for
    /// zero-length input or 0 when absent.
    pub fn search_string(&self, search: i32) -> i32 {
        let len = self.length(search);
        if len == 0 {
            return EMPTY_STRING;
        }
        let mut s = search - 1;
        while s > 65535 {
            if self.length(s) == len && self.str_eq_str(s, search) {
                return s;
            }
            s -= 1;
        }
        0
    }

    /// Seals the current pool text and returns an existing equal string when
    /// one exists (`slow_make_string`); otherwise returns the new string.
    pub fn slow_make_string(&mut self) -> Result<i32, TeXMemoryError> {
        let t = self.make_string()?;
        let s = self.search_string(t);
        if s > 0 {
            self.str_ptr -= 1;
            self.pool_ptr = self.str_start[(self.str_ptr - TOO_BIG_CHAR) as usize];
            self.str_pool.truncate(self.pool_ptr as usize);
            return Ok(s);
        }
        Ok(t)
    }

    /// Discards pool string `s` when it is the most recently allocated one
    /// (`flush_str`).
    pub fn flush_str(&mut self, s: i32) {
        if s == self.str_ptr - 1 {
            self.str_ptr -= 1;
            self.pool_ptr = self.str_start[(self.str_ptr - TOO_BIG_CHAR) as usize];
            self.str_pool.truncate(self.pool_ptr as usize);
        }
    }

    /// Loads the fixed string constants (`load_pool_strings`); returns the
    /// last string number allocated, or 0 when `spare_size` is too small.
    pub fn load_pool_strings(&mut self, spare_size: i32) -> i32 {
        let mut total_len = 0i32;
        let mut g = 0i32;
        for s in STRING_CONSTANTS {
            total_len += s.len() as i32;
            if total_len >= spare_size {
                return 0;
            }
            for &b in s.as_bytes() {
                self.pool_push(i32::from(b));
            }
            g = self.make_string().unwrap_or(0);
        }
        g
    }

    /// Resets the pool and loads the constant strings (`get_strings_started`).
    pub fn get_strings_started(&mut self) {
        self.pool_ptr = 0;
        self.str_pool.clear();
        self.str_ptr = TOO_BIG_CHAR;
        self.str_start.clear();
        self.str_start.push(0);
        self.load_pool_strings(self.pool_size - self.string_vacancies);
    }
}
