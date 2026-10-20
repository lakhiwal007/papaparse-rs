'use strict';
// The `Papa` API on top of the Rust core, shared by the wasm and Node-API packages.
//
// Parsing, delimiter detection, headers and typing run in Rust. What PapaParse does in
// JavaScript around its parser — the streamers that cut input into chunks, `step` / `chunk`
// / `complete` callbacks, pause / resume / abort — is ported here nearly verbatim from
// papaparse.js 5.7.0 (MIT, © Matthew Holt), so callbacks see exactly what PapaParse gives them.

const { decode, rowDecoder } = require('./decode.cjs');

const RECORD_SEP = String.fromCharCode(30);
const UNIT_SEP = String.fromCharCode(31);
const BYTE_ORDER_MARK = '﻿';
const BAD_DELIMITERS = ['\r', '\n', '"', BYTE_ORDER_MARK];
const NODE_STREAM_INPUT = 1;

// `unparse` cell kinds; must match `kind` in src/unparse.rs.
const STRING = 0, NUMBER = 1, TRUE = 2, FALSE = 3, EMPTY = 4, DATE = 5, OTHER = 6;

/**
 * Builds the `Papa` object around a native binding:
 *   parseTape(input, config) → tape                                   one-shot fast path
 *   newSession(config) → { parse(input, base, ignoreLastRow, transformHeader, onRows) → tape, free() }
 *     where onRows(batch) gets step rows as a tape plus per-row cursors (see src/session.rs)
 *   guessLineEndings(input, quoteChar) → '\n' | '\r' | '\r\n'
 *   unparse(fields, text, lens, kinds, numbers, rowLens, rowWidths, options, quotes, escapeFormulae) → string
 */
function createPapa(native) {
	const Papa = {};
	Papa.parse = CsvToJson;
	Papa.unparse = JsonToCsv;
	Papa.RECORD_SEP = RECORD_SEP;
	Papa.UNIT_SEP = UNIT_SEP;
	Papa.BYTE_ORDER_MARK = BYTE_ORDER_MARK;
	Papa.BAD_DELIMITERS = BAD_DELIMITERS;
	Papa.WORKERS_SUPPORTED = false; // `worker: true` parses on the calling thread
	Papa.NODE_STREAM_INPUT = NODE_STREAM_INPUT;
	Papa.LocalChunkSize = 1024 * 1024 * 10;
	Papa.RemoteChunkSize = 1024 * 1024 * 5;
	Papa.DefaultDelimiter = ',';
	Papa.ParserHandle = ParserHandle;
	Papa.StringStreamer = StringStreamer;
	Papa.ReadableStreamStreamer = ReadableStreamStreamer;
	Papa.DuplexStreamStreamer = DuplexStreamStreamer;

	// Frees a wasm session whose parse was abandoned (e.g. paused and never resumed).
	const sessions = typeof FinalizationRegistry === 'function'
		? new FinalizationRegistry((session) => session.free())
		: null;

	function CsvToJson(_input, _config) {
		_config = _config || {};
		var dynamicTyping = _config.dynamicTyping || false;
		if (isFunction(dynamicTyping)) {
			_config.dynamicTypingFunction = dynamicTyping;
			// Will be filled on first row call
			dynamicTyping = {};
		}
		_config.dynamicTyping = dynamicTyping;

		_config.transform = isFunction(_config.transform) ? _config.transform : false;

		if (_config.downloadTimeout !== undefined) {
			var downloadTimeout = parseInt(_config.downloadTimeout);
			if (isNaN(downloadTimeout)) {
				throw new Error('Config downloadTimeout value (' + _config.downloadTimeout + ') not parsable by parseInt(val).');
			}
			_config.downloadTimeout = downloadTimeout;
		}

		var streamer = null;
		if (_input === NODE_STREAM_INPUT) {
			// create a node Duplex stream for use with .pipe
			streamer = new DuplexStreamStreamer(_config);
			return streamer.getStream();
		} else if (typeof _input === 'string') {
			if (_config.download) throw new Error('papaparse: `download` needs a browser; fetch the text and parse it instead');
			// Rust strips the BOM itself on this path.
			if (isOneShot(_config)) return parseOneShot(_input, _config);
			_input = stripBom(_input);
			streamer = new StringStreamer(_config);
		} else if (_input && _input.readable === true && isFunction(_input.read) && isFunction(_input.on)) {
			streamer = new ReadableStreamStreamer(_config);
		} else {
			throw new TypeError('papaparse: input must be a string, a Node.js readable stream or Papa.NODE_STREAM_INPUT');
		}
		return streamer.stream(_input);
	}

	/** No callback can observe the difference: parse in one native call. */
	function isOneShot(config) {
		return !isFunction(config.step) && !isFunction(config.chunk) && !config.chunkSize
			&& !isFunction(config.beforeFirstChunk) && !config.transform
			&& !config.dynamicTypingFunction && !isFunction(config.transformHeader)
			&& !isFunction(config.delimiter);
	}

	function parseOneShot(input, config) {
		const results = decode(input, native.parseTape(input, nativeConfig(config, true)));
		if (isFunction(config.complete)) config.complete(results, null);
		return results;
	}

	/** ChunkStreamer is the base prototype for various streamer implementations. */
	function ChunkStreamer(config) {
		this._handle = null;
		this._finished = false;
		this._completed = false;
		this._halted = false;
		this._input = null;
		this._baseIndex = 0;
		this._partialLine = '';
		this._rowCount = 0;
		this._start = 0;
		this._nextChunk = null;
		this.isFirstChunk = true;
		this._completeResults = {
			data: [],
			errors: [],
			meta: {}
		};
		replaceConfig.call(this, config);

		this.parseChunk = function(chunk, isFakeChunk) {
			// First chunk pre-processing
			const skipFirstNLines = parseInt(this._config.skipFirstNLines) || 0;
			if (this.isFirstChunk && skipFirstNLines > 0) {
				let _newline = this._config.newline;
				if (!_newline) {
					const quoteChar = this._config.quoteChar || '"';
					_newline = this._handle.guessLineEndings(chunk, quoteChar);
				}
				const splitChunk = chunk.split(_newline);
				chunk = [...splitChunk.slice(skipFirstNLines)].join(_newline);
			}
			if (this.isFirstChunk && isFunction(this._config.beforeFirstChunk)) {
				var modifiedChunk = this._config.beforeFirstChunk(chunk);
				if (modifiedChunk !== undefined)
					chunk = modifiedChunk;
			}
			this.isFirstChunk = false;
			this._halted = false;

			// Rejoin the line we likely just split in two by chunking the file
			var aggregate = this._partialLine + chunk;
			this._partialLine = '';
			var results = this._handle.parse(aggregate, this._baseIndex, !this._finished);

			if (this._handle.paused() || this._handle.aborted()) {
				this._halted = true;
				return;
			}

			var lastIndex = results.meta.cursor;

			if (!this._finished) {
				this._partialLine = aggregate.substring(lastIndex - this._baseIndex);
				this._baseIndex = lastIndex;
			}

			if (results && results.data)
				this._rowCount += results.data.length;

			var finishedIncludingPreview = this._finished || (this._config.preview && this._rowCount >= this._config.preview);
			if (finishedIncludingPreview) this._handle.dispose();

			if (isFunction(this._config.chunk) && !isFakeChunk) {
				this._config.chunk(results, this._handle);
				if (this._handle.paused() || this._handle.aborted()) {
					this._halted = true;
					return;
				}
				results = undefined;
				this._completeResults = undefined;
			}

			if (!this._config.step && !this._config.chunk) {
				this._completeResults.data = this._completeResults.data.concat(results.data);
				this._completeResults.errors = this._completeResults.errors.concat(results.errors);
				this._completeResults.meta = results.meta;
			}

			if (!this._completed && finishedIncludingPreview && isFunction(this._config.complete) && (!results || !results.meta.aborted)) {
				this._config.complete(this._completeResults, this._input);
				this._completed = true;
			}

			if (!finishedIncludingPreview && (!results || !results.meta.paused))
				this._nextChunk();

			return results;
		};

		this._sendError = function(error) {
			if (isFunction(this._config.error))
				this._config.error(error);
		};

		function replaceConfig(config) {
			// Deep-copy the config so we can edit it
			var configCopy = copy(config);
			configCopy.chunkSize = parseInt(configCopy.chunkSize);	// parseInt VERY important so we don't concatenate strings!
			if (!config.step && !config.chunk)
				configCopy.chunkSize = null;  // disable Range header if not streaming; bad values break IIS - see issue #196
			this._handle = new ParserHandle(configCopy);
			this._handle.streamer = this;
			this._config = configCopy;	// persist the copy to the caller
		}
	}

	function StringStreamer(config) {
		config = config || {};
		ChunkStreamer.call(this, config);

		var remaining;
		this.stream = function(s) {
			remaining = s;
			return this._nextChunk();
		};
		this._nextChunk = function() {
			if (this._finished) return;
			var size = this._config.chunkSize;
			var chunk;
			if (size) {
				chunk = remaining.substring(0, size);
				remaining = remaining.substring(size);
			} else {
				chunk = remaining;
				remaining = '';
			}
			this._finished = !remaining;
			return this.parseChunk(chunk);
		};
	}
	StringStreamer.prototype = Object.create(ChunkStreamer.prototype);
	StringStreamer.prototype.constructor = StringStreamer;

	function ReadableStreamStreamer(config) {
		config = config || {};

		ChunkStreamer.call(this, config);

		var queue = [];
		var parseOnData = true;
		var streamHasEnded = false;

		this.stream = function(stream) {
			this._input = stream;

			this._input.on('data', this._streamData);
			this._input.on('end', this._streamEnd);
			this._input.on('error', this._streamError);
		};

		this._checkIsFinished = function() {
			if (streamHasEnded && queue.length === 1) {
				this._finished = true;
			}
		};

		this._nextChunk = function() {
			this._checkIsFinished();
			if (queue.length) {
				this.parseChunk(queue.shift());
			} else {
				parseOnData = true;
			}
		};

		this._streamData = bindFunction(function(chunk) {
			try {
				queue.push(typeof chunk === 'string' ? chunk : chunk.toString(this._config.encoding));

				if (parseOnData) {
					parseOnData = false;
					this._checkIsFinished();
					this.parseChunk(queue.shift());
				}
			} catch (error) {
				this._streamError(error);
			}
		}, this);

		this._streamError = bindFunction(function(error) {
			this._streamCleanUp();
			this._sendError(error);
		}, this);

		this._streamEnd = bindFunction(function() {
			this._streamCleanUp();
			streamHasEnded = true;
			this._streamData('');
		}, this);

		this._streamCleanUp = bindFunction(function() {
			this._input.removeListener('data', this._streamData);
			this._input.removeListener('end', this._streamEnd);
			this._input.removeListener('error', this._streamError);
		}, this);
	}
	ReadableStreamStreamer.prototype = Object.create(ChunkStreamer.prototype);
	ReadableStreamStreamer.prototype.constructor = ReadableStreamStreamer;

	function DuplexStreamStreamer(_config) {
		var Duplex = require('stream').Duplex;
		var config = copy(_config);
		var parseOnWrite = true;
		var writeStreamHasFinished = false;
		var parseCallbackQueue = [];
		var stream = null;

		this._onCsvData = function(results) {
			var data = results.data;
			if (!stream.push(data) && !this._handle.paused()) {
				// the writeable consumer buffer has filled up so we need to pause until
				// more items can be processed
				this._handle.pause();
			}
		};

		this._onCsvComplete = function() {
			// node will finish the read stream when null is pushed
			stream.push(null);
		};

		config.step = bindFunction(this._onCsvData, this);
		config.complete = bindFunction(this._onCsvComplete, this);
		ChunkStreamer.call(this, config);

		this._nextChunk = function() {
			if (writeStreamHasFinished && parseCallbackQueue.length === 1) {
				this._finished = true;
			}
			if (parseCallbackQueue.length) {
				parseCallbackQueue.shift()();
			} else {
				parseOnWrite = true;
			}
		};

		this._addToParseQueue = function(chunk, callback) {
			// add to queue so that we can indicate completion via callback; node will
			// automatically pause the incoming stream when too many items have been added
			// without their callback being invoked
			parseCallbackQueue.push(bindFunction(function() {
				this.parseChunk(typeof chunk === 'string' ? chunk : chunk.toString(config.encoding));
				if (isFunction(callback)) {
					return callback();
				}
			}, this));
			if (parseOnWrite) {
				parseOnWrite = false;
				this._nextChunk();
			}
		};

		this._onRead = function() {
			if (this._handle.paused()) {
				// the writeable consumer can handle more data so resume the chunk parsing
				this._handle.resume();
			}
		};

		this._onWrite = function(chunk, encoding, callback) {
			this._addToParseQueue(chunk, callback);
		};

		this._onWriteComplete = function() {
			writeStreamHasFinished = true;
			// have to write empty string so parser knows its done
			this._addToParseQueue('');
		};

		this.getStream = function() {
			return stream;
		};
		stream = new Duplex({
			readableObjectMode: true,
			decodeStrings: false,
			read: bindFunction(this._onRead, this),
			write: bindFunction(this._onWrite, this)
		});
		stream.once('finish', bindFunction(this._onWriteComplete, this));
	}
	DuplexStreamStreamer.prototype = Object.create(ChunkStreamer.prototype);
	DuplexStreamStreamer.prototype.constructor = DuplexStreamStreamer;

	/**
	 * PapaParse's `ParserHandle`. Rust does the parsing, the header row, empty-line skipping,
	 * dynamic typing and the per-row bookkeeping of `step`; this keeps the JS-visible state
	 * (`pause`, `resume`, `abort`) and runs `transform` / a `dynamicTyping` function, whose
	 * results may be any JS value.
	 */
	function ParserHandle(_config) {
		var MAX_FLOAT = Math.pow(2, 53);
		var MIN_FLOAT = -MAX_FLOAT;
		var FLOAT = /^\s*-?(\d+\.?|\.\d+|\d+\.\d+)([eE][-+]?\d+)?\s*$/;
		var ISO_DATE = /^((\d{4}-[01]\d-[0-3]\dT[0-2]\d:[0-5]\d:[0-5]\d\.\d+([+-][0-2]\d:[0-5]\d|Z))|(\d{4}-[01]\d-[0-3]\dT[0-2]\d:[0-5]\d:[0-5]\d([+-][0-2]\d:[0-5]\d|Z))|(\d{4}-[01]\d-[0-3]\dT[0-2]\d:[0-5]\d([+-][0-2]\d:[0-5]\d|Z)))$/;
		var self = this;
		var _session = null;
		var _parsing = false;	// Inside a native parse: freeing the session must wait
		var _disposed = false;
		var _paused = false;
		var _aborted = false;
		var _input = '';
		var _charIndex = 0;		// Offset in `_input` just past the last row (JS `getCharIndex`)
		var _fields = [];		// One array shared by every result's `meta.fields`, as in PapaParse
		var _results = {
			data: [],
			errors: [],
			meta: {}
		};
		// `transform` and a `dynamicTyping` function may return any value, so typing runs here.
		var _jsTyping = !!(_config.transform || _config.dynamicTypingFunction);
		var post = _jsTyping ? function(value, field) {
			if (_config.transform)
				value = _config.transform(value, field);
			return parseDynamic(field, value);
		} : null;
		var transformHeader = isFunction(_config.transformHeader)
			? function(header, i) { return String(_config.transformHeader(header, i)); }
			: undefined;
		var onRows = isFunction(_config.step) ? stepRows : undefined;

		this.parse = function(input, baseIndex, ignoreLastRow) {
			if (_disposed && !input) {
				// A finished or aborted parse resumed with nothing left (as PapaParse allows).
				_results = { data: [], errors: [], meta: _results.meta };
				return _paused ? { meta: { paused: true } } : _results;
			}
			if (!_session) {
				if (isFunction(_config.delimiter))
					_config.delimiter = _config.delimiter(input);
				var config = nativeConfig(_config, false);
				if (_jsTyping) config.dynamicTyping = false;
				_session = native.newSession(config);
				if (sessions) sessions.register(this, _session, this);
			}
			_input = input;
			_charIndex = 0;
			var tape;
			_parsing = true;
			try {
				tape = _session.parse(input, baseIndex || 0, !!ignoreLastRow, transformHeader, onRows);
			} finally {
				_parsing = false;
				if (_disposed) this.dispose();
			}
			_results = decode(input, tape, post);
			if (_config.header)
				_results.meta.fields = shareFields(_results.meta.fields);
			return _paused ? { meta: { paused: true } } : _results;
		};

		this.paused = function() {
			return _paused;
		};

		this.pause = function() {
			_paused = true;
			// If it is streaming via "chunking", the reader will start appending correctly already so no need to substring,
			// otherwise we can get duplicate content within a row
			_input = isFunction(_config.chunk) ? '' : _input.substring(_charIndex);
		};

		this.resume = function() {
			if (self.streamer._halted) {
				_paused = false;
				self.streamer.parseChunk(_input, true);
			} else {
				// Bugfix: #636 In case the processing hasn't halted yet
				// wait for it to halt in order to resume
				setTimeout(self.resume, 3);
			}
		};

		this.aborted = function() {
			return _aborted;
		};

		this.abort = function() {
			_aborted = true;
			_results.meta.aborted = true;
			if (isFunction(_config.complete))
				_config.complete(_results);
			_input = '';
			this.dispose();
		};

		this.guessLineEndings = function(input, quoteChar) {
			return native.guessLineEndings(input, quoteChar);
		};

		/** Releases native memory once no more chunks will come. */
		this.dispose = function() {
			_disposed = true;
			if (_session && !_parsing) {
				if (sessions) sessions.unregister(this);
				var session = _session;
				_session = null;
				session.free();
			}
		};

		/**
		 * Called from Rust with a batch of rows in step mode. Rows are decoded one at a time,
		 * right before their `step` call, so `transform` never sees rows after an abort.
		 * Returns -1 (all delivered), -2 (aborted) or the index of the row paused after.
		 */
		function stepRows(batch) {
			var meta = JSON.parse(batch.meta);
			var fields = _config.header ? shareFields(meta.fields) : null;
			var next = rowDecoder(_input, batch, fields, post);
			var errors = batch.errors ? JSON.parse(batch.errors) : null;
			var rows = batch.rowLens.length;
			for (var i = 0; i < rows; i++) {
				_charIndex = batch.charIndexes[i];
				var rowMeta = {
					delimiter: meta.delimiter,
					linebreak: meta.linebreak,
					aborted: meta.aborted,
					truncated: meta.truncated,
					cursor: batch.cursors[i],
					renamedHeaders: meta.renamedHeaders
				};
				if (fields) rowMeta.fields = fields;
				_results = { data: next(), errors: (errors && errors[i]) || [], meta: rowMeta };
				_config.step(_results, self);
				if (_aborted) return -2;
				if (_paused) return i;
			}
			return -1;
		}

		/** PapaParse fills one `_fields` array once and hands out references to it. */
		function shareFields(fields) {
			if (!_fields.length && fields)
				for (var i = 0; i < fields.length; i++) _fields.push(fields[i]);
			return _fields;
		}

		function testFloat(s) {
			if (FLOAT.test(s)) {
				var floatValue = parseFloat(s);
				if (floatValue > MIN_FLOAT && floatValue < MAX_FLOAT) {
					return true;
				}
			}
			return false;
		}

		function shouldApplyDynamicTyping(field) {
			// Cache function values to avoid calling it for each row
			if (_config.dynamicTypingFunction && _config.dynamicTyping[field] === undefined) {
				_config.dynamicTyping[field] = _config.dynamicTypingFunction(field);
			}
			return (_config.dynamicTyping[field] || _config.dynamicTyping) === true;
		}

		function parseDynamic(field, value) {
			if (shouldApplyDynamicTyping(field)) {
				if (value === 'true' || value === 'TRUE')
					return true;
				else if (value === 'false' || value === 'FALSE')
					return false;
				else if (testFloat(value))
					return parseFloat(value);
				else if (ISO_DATE.test(value))
					return new Date(value);
				else
					return (value === '' ? null : value);
			}
			return value;
		}
	}

	/** Port of `JsonToCsv`: resolves the input shape here, writes the CSV in Rust. */
	function JsonToCsv(_input, _config) {
		var _quotes = false;
		var _writeHeader = true;
		var _delimiter = ',';
		var _newline = '\r\n';
		var _quoteChar = '"';
		var _escapeChar;
		var _skipEmptyLines = false;
		var _columns = null;
		var _escapeFormulae = false;

		unpackConfig();

		if (typeof _input === 'string')
			_input = JSON.parse(_input);

		if (Array.isArray(_input)) {
			if (!_input.length || Array.isArray(_input[0]))
				return serialize(null, _input);
			else if (typeof _input[0] === 'object')
				return serialize(_columns || Object.keys(_input[0]), _input);
		} else if (typeof _input === 'object') {
			if (typeof _input.data === 'string')
				_input.data = JSON.parse(_input.data);

			if (Array.isArray(_input.data)) {
				if (!_input.fields)
					_input.fields = _input.meta && _input.meta.fields || _columns;

				if (!_input.fields)
					_input.fields = Array.isArray(_input.data[0])
						? _input.fields
						: typeof _input.data[0] === 'object'
							? Object.keys(_input.data[0])
							: [];

				if (!(Array.isArray(_input.data[0])) && typeof _input.data[0] !== 'object')
					_input.data = [_input.data];	// handles input like [1,2,3] or ['asdf']
			}

			return serialize(_input.fields || [], _input.data || []);
		}

		// Default (any valid paths should return before this)
		throw new Error('Unable to serialize unrecognized input');

		function unpackConfig() {
			if (typeof _config !== 'object')
				return;

			if (typeof _config.delimiter === 'string'
				&& !BAD_DELIMITERS.filter(function(value) { return _config.delimiter.indexOf(value) !== -1; }).length) {
				_delimiter = _config.delimiter;
			}

			if (typeof _config.quotes === 'boolean'
				|| typeof _config.quotes === 'function'
				|| Array.isArray(_config.quotes))
				_quotes = _config.quotes;

			if (typeof _config.skipEmptyLines === 'boolean'
				|| typeof _config.skipEmptyLines === 'string')
				_skipEmptyLines = _config.skipEmptyLines;

			if (typeof _config.newline === 'string')
				_newline = _config.newline;

			if (typeof _config.quoteChar === 'string')
				_quoteChar = _config.quoteChar;

			if (typeof _config.header === 'boolean')
				_writeHeader = _config.header;

			if (Array.isArray(_config.columns)) {
				if (_config.columns.length === 0) throw new Error('Option columns is empty');
				_columns = _config.columns;
			}

			if (_config.escapeChar !== undefined)
				_escapeChar = String(_config.escapeChar);

			if (_config.escapeFormulae instanceof RegExp) {
				_escapeFormulae = _config.escapeFormulae;
			} else if (typeof _config.escapeFormulae === 'boolean' && _config.escapeFormulae) {
				_escapeFormulae = true;
			}
		}

		/** Flattens the cells `serialize` would write, then writes them in Rust. */
		function serialize(fields, data) {
			if (typeof fields === 'string')
				fields = JSON.parse(fields);
			if (typeof data === 'string')
				data = JSON.parse(data);

			var hasHeader = Array.isArray(fields) && fields.length > 0;
			var dataKeyedByField = !(Array.isArray(data[0]));

			var texts = [], lens = [], kinds = [], numbers = [];
			function push(value) {
				if (value === undefined || value === null) {
					kinds.push(EMPTY);
				} else if (value.constructor === Date) {
					// An invalid date is written empty but is not an empty line ("Invalid Date").
					var iso = isNaN(value.getTime()) ? '' : value.toISOString();
					kinds.push(DATE); texts.push(iso); lens.push(iso.length);
				} else if (typeof value === 'string') {
					kinds.push(STRING); texts.push(value); lens.push(value.length);
				} else if (typeof value === 'number') {
					kinds.push(NUMBER); numbers.push(value);
				} else if (typeof value === 'boolean') {
					kinds.push(value ? TRUE : FALSE);
				} else {
					var text = value.toString();
					kinds.push(OTHER); texts.push(text); lens.push(text.length);
				}
			}

			var rowLens = new Uint32Array(data.length);
			var rowWidths = new Uint32Array(data.length);
			for (var row = 0; row < data.length; row++) {
				var maxCol = hasHeader ? fields.length : data[row].length;
				rowWidths[row] = hasHeader ? Object.keys(data[row]).length : data[row].length;
				rowLens[row] = maxCol || 0;
				for (var col = 0; col < maxCol; col++) {
					var colIdx = hasHeader && dataKeyedByField ? fields[col] : col;
					push(data[row][colIdx]);
				}
			}

			var options = {
				quotes: _quotes === true,
				quoteColumns: Array.isArray(_quotes) ? Array.from(_quotes, Boolean) : undefined,
				header: _writeHeader,
				delimiter: _delimiter,
				newline: _newline,
				quoteChar: _quoteChar,
				escapeChar: _escapeChar,
				skipEmptyLines: _skipEmptyLines,
				escapeFormulae: _escapeFormulae === true,
			};
			// Rust says which cell; the callback gets its original value, as in PapaParse.
			var quotes = isFunction(_quotes)
				? function(col, row) {
					var value = row < 0 ? fields[col] : data[row][hasHeader && dataKeyedByField ? fields[col] : col];
					return !!_quotes(value, col);
				}
				: undefined;
			var escapeFormulae = _escapeFormulae instanceof RegExp
				? function(text) { return _escapeFormulae.test(text); }
				: undefined;
			var header = hasHeader ? fields.map(function(f) { return f == null ? '' : String(f); }) : [];
			return native.unparse(header, texts.join(''), Uint32Array.from(lens), Uint8Array.from(kinds),
				Float64Array.from(numbers), rowLens, rowWidths, options, quotes, escapeFormulae);
		}
	}

	/** The options Rust reads, with PapaParse's type checks applied (invalid ones are ignored). */
	function nativeConfig(c, withSkip) {
		var o = {};
		if (typeof c.delimiter === 'string') o.delimiter = c.delimiter;
		if (typeof c.newline === 'string') o.newline = c.newline;
		if (typeof c.quoteChar === 'string') o.quoteChar = c.quoteChar;
		if (typeof c.escapeChar === 'string') o.escapeChar = c.escapeChar;
		if (c.header) o.header = true;
		if (typeof c.dynamicTyping === 'boolean') {
			o.dynamicTyping = c.dynamicTyping;
		} else if (c.dynamicTyping && typeof c.dynamicTyping === 'object') {
			var fields = {};
			for (var key in c.dynamicTyping) fields[key] = c.dynamicTyping[key] === true;
			o.dynamicTyping = fields;
		}
		if (typeof c.preview === 'number') o.preview = c.preview;
		if (typeof c.comments === 'boolean' || typeof c.comments === 'string') o.comments = c.comments;
		if (typeof c.skipEmptyLines === 'boolean' || typeof c.skipEmptyLines === 'string') o.skipEmptyLines = c.skipEmptyLines;
		if (withSkip && c.skipFirstNLines !== undefined) o.skipFirstNLines = parseInt(c.skipFirstNLines) || 0;
		if (typeof c.fastMode === 'boolean') o.fastMode = c.fastMode;
		if (Array.isArray(c.delimitersToGuess)) o.delimitersToGuess = c.delimitersToGuess.filter(function(d) { return typeof d === 'string'; });
		return o;
	}

	return Papa;
}

// Strip character from UTF-8 BOM encoded files that cause issue parsing the file
function stripBom(string) {
	if (string.charCodeAt(0) === 0xfeff) {
		return string.slice(1);
	}
	return string;
}

function copy(obj) {
	if (typeof obj !== 'object' || obj === null)
		return obj;
	var cpy = Array.isArray(obj) ? [] : {};
	for (var key in obj)
		cpy[key] = copy(obj[key]);
	return cpy;
}

function bindFunction(f, self) {
	return function() { f.apply(self, arguments); };
}

function isFunction(func) {
	return typeof func === 'function';
}

module.exports = { createPapa };
